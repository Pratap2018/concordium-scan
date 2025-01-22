//! TODO
//! - Enable GraphiQL through flag instead of always.

#![allow(unused_variables)]

mod account;
mod account_metrics;
mod baker;
mod block;
mod block_metrics;
mod contract;
mod module_reference_event;
mod token;
mod transaction;
mod transaction_metrics;

// TODO remove this macro, when done with first iteration
/// Short hand for returning API error with the message not implemented.
macro_rules! todo_api {
    () => {
        Err(crate::graphql_api::ApiError::InternalError(String::from("Not implemented")))
    };
}
pub(crate) use todo_api;

use crate::{
    scalar_types::{
        BlockHeight, DateTime, RewardPeriodLength as ScalarRewardPeriodLength, TimeSpan,
    },
    transaction_event::smart_contracts::InvalidContractVersionError,
};
use account::Account;
use anyhow::Context as _;
use async_graphql::{
    http::GraphiQLSource,
    types::{self, connection},
    Context, EmptyMutation, Enum, MergedObject, Object, Schema, SimpleObject, Subscription,
};
use async_graphql_axum::GraphQLSubscription;
use block::Block;
use chrono::{Duration, TimeDelta};
use concordium_rust_sdk::{
    base::contracts_common::schema::VersionedSchemaError, id::types as sdk_types,
    types::RewardPeriodLength,
};
use derive_more::Display;
use futures::prelude::*;
use prometheus_client::registry::Registry;
use regex::Regex;
use sqlx::PgPool;
use std::{
    cmp::{max, min},
    error::Error,
    str::FromStr,
    sync::Arc,
};
use token::Token;
use tokio::{net::TcpListener, sync::broadcast};
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_util::sync::CancellationToken;
use tower_http::cors::{Any, CorsLayer};
use tracing::{error, info};
use transaction::Transaction;

const VERSION: &str = clap::crate_version!();

#[derive(clap::Args)]
pub struct ApiServiceConfig {
    /// Account(s) that should not be considered in circulation.
    #[arg(long, env = "CCDSCAN_API_CONFIG_NON_CIRCULATING_ACCOUNTS", value_delimiter = ',')]
    non_circulating_account: Vec<sdk_types::AccountAddress>,
    /// The most transactions which can be queried at once.
    #[arg(long, env = "CCDSCAN_API_CONFIG_TRANSACTION_CONNECTION_LIMIT", default_value = "100")]
    transaction_connection_limit: u64,
    #[arg(long, env = "CCDSCAN_API_CONFIG_BLOCK_CONNECTION_LIMIT", default_value = "100")]
    block_connection_limit: u64,
    #[arg(long, env = "CCDSCAN_API_CONFIG_ACCOUNT_CONNECTION_LIMIT", default_value = "100")]
    account_connection_limit: u64,
    #[arg(
        long,
        env = "CCDSCAN_API_CONFIG_ACCOUNT_STATEMENTS_CONNECTION_LIMIT",
        default_value = "100"
    )]
    account_statements_connection_limit: u64,
    #[arg(
        long,
        env = "CCDSCAN_API_CONFIG_ACCOUNT_SCHEDULE_CONNECTION_LIMIT",
        default_value = "100"
    )]
    account_schedule_connection_limit: u64,
    #[arg(long, env = "CCDSCAN_API_CONFIG_CONTRACT_CONNECTION_LIMIT", default_value = "100")]
    contract_connection_limit: u64,
    #[arg(
        long,
        env = "CCDSCAN_API_CONFIG_TRANSACTION_EVENT_CONNECTION_LIMIT",
        default_value = "100"
    )]
    transaction_event_connection_limit: u64,
    #[arg(long, env = "CCDSCAN_API_CONFIG_TOKENS_CONNECTION_LIMIT", default_value = "100")]
    tokens_connection_limit: u64,
    #[arg(
        long,
        env = "CCDSCAN_API_CONFIG_CONTRACT_TOKENS_COLLECTION_LIMIT",
        default_value = "100"
    )]
    contract_tokens_collection_limit: u64,
    #[arg(
        long,
        env = "CCDSCAN_API_CONFIG_CONTRACT_EVENTS_COLLECTION_LIMIT",
        default_value = "100"
    )]
    contract_events_collection_limit: u64,
    #[arg(
        long,
        env = "CCDSCAN_API_CONFIG_CONTRACT_REJECT_EVENTS_COLLECTION_LIMIT",
        default_value = "100"
    )]
    contract_reject_events_collection_limit: u64,
    #[arg(
        long,
        env = "CCDSCAN_API_CONFIG_MODULE_REFERENCE_REJECT_EVENTS_COLLECTION_LIMIT",
        default_value = "100"
    )]
    module_reference_reject_events_collection_limit: u64,
    #[arg(
        long,
        env = "CCDSCAN_API_CONFIG_MODULE_REFERENCE_LINKED_CONTRACTS_COLLECTION_LIMIT",
        default_value = "100"
    )]
    module_reference_linked_contracts_collection_limit: u64,
    #[arg(
        long,
        env = "CCDSCAN_API_CONFIG_MODULE_REFERENCE_CONTRACT_LINK_EVENTS_COLLECTION_LIMIT",
        default_value = "100"
    )]
    module_reference_contract_link_events_collection_limit: u64,
    #[arg(long, env = "CCDSCAN_API_CONFIG_REWARD_CONNECTION_LIMIT", default_value = "100")]
    reward_connection_limit: u64,
    #[arg(
        long,
        env = "CCDSCAN_API_CONFIG_TOKEN_HOLDER_ADDRESSES_COLLECTION_LIMIT",
        default_value = "100"
    )]
    token_holder_addresses_collection_limit: u64,
    #[arg(long, env = "CCDSCAN_API_CONFIG_TOKEN_EVENTS_COLLECTION_LIMIT", default_value = "100")]
    token_events_collection_limit: u64,
}

#[derive(MergedObject, Default)]
pub struct Query(
    BaseQuery,
    baker::QueryBaker,
    block::QueryBlocks,
    transaction::QueryTransactions,
    account::QueryAccounts,
    account_metrics::QueryAccountMetrics,
    transaction_metrics::QueryTransactionMetrics,
    block_metrics::QueryBlockMetrics,
    module_reference_event::QueryModuleReferenceEvent,
    contract::QueryContract,
    token::QueryToken,
);

/// This state is not persisted in the database and will be re-created at
/// start-up of this service.
#[derive(Copy, Clone)]
pub struct MemoryState {
    // Duration of an epoch of the current consensus algorithm.
    // E.g. This value is 1 hour for testnet in protocol version 8.
    pub current_epoch_duration:       TimeDelta,
    // Number of epochs between reward payout to happen.
    // E.g. This value is 24 for testnet in protocol version 8.
    // E.g. This means after 24 hours a new payday block is happening on testnet with reward
    // payouts.
    pub current_reward_period_length: RewardPeriodLength,
}

/// The service config with state in memory and the api configuration.
pub struct ServiceConfig {
    pub api_config:   ApiServiceConfig,
    pub memory_state: MemoryState,
}

pub struct Service {
    pub schema: Schema<Query, EmptyMutation, Subscription>,
}
impl Service {
    pub fn new(
        subscription: Subscription,
        registry: &mut Registry,
        pool: PgPool,
        config: ServiceConfig,
    ) -> Self {
        let schema = Schema::build(Query::default(), EmptyMutation, subscription)
            .extension(async_graphql::extensions::Tracing)
            .extension(monitor::MonitorExtension::new(registry))
            .data(pool)
            .data(config)
            .finish();
        Self {
            schema,
        }
    }

    pub async fn serve(
        self,
        tcp_listener: TcpListener,
        stop_signal: CancellationToken,
    ) -> anyhow::Result<()> {
        let cors_layer = CorsLayer::new()
            .allow_origin(Any)  // Open access to selected route
            .allow_methods(Any)
            .allow_headers(Any);
        let app = axum::Router::new()
            .route("/", axum::routing::get(Self::graphiql))
            .route(
                "/api/graphql",
                axum::routing::post_service(async_graphql_axum::GraphQL::new(self.schema.clone())),
            )
            .route_service("/ws/graphql", GraphQLSubscription::new(self.schema))
            .layer(cors_layer);

        axum::serve(tcp_listener, app)
            .with_graceful_shutdown(stop_signal.cancelled_owned())
            .await?;
        Ok(())
    }

    async fn graphiql() -> impl axum::response::IntoResponse {
        axum::response::Html(
            GraphiQLSource::build()
                .endpoint("/api/graphql")
                .subscription_endpoint("/ws/graphql")
                .finish(),
        )
    }
}
/// Module containing types and logic for building an async_graphql extension
/// which allows for monitoring of the service.
mod monitor {
    use async_graphql::async_trait::async_trait;
    use futures::prelude::*;
    use prometheus_client::{
        encoding::EncodeLabelSet,
        metrics::{
            counter::Counter,
            family::Family,
            gauge::Gauge,
            histogram::{self, Histogram},
        },
        registry::Registry,
    };
    use std::sync::Arc;
    use tokio::time::Instant;

    /// Type representing the Prometheus labels used for metrics related to
    /// queries.
    #[derive(Debug, Clone, EncodeLabelSet, PartialEq, Eq, Hash)]
    struct QueryLabels {
        /// Identifier of the top level query.
        query: String,
    }
    /// Extension for async_graphql adding monitoring.
    #[derive(Clone)]
    pub struct MonitorExtension {
        /// Metric for tracking current number of requests in-flight.
        in_flight_requests:   Family<QueryLabels, Gauge>,
        /// Metric for counting total number of requests.
        total_requests:       Family<QueryLabels, Counter>,
        /// Metric for collecting execution duration for requests.
        request_duration:     Family<QueryLabels, Histogram>,
        /// Metric tracking current open subscriptions.
        active_subscriptions: Gauge,
    }
    impl MonitorExtension {
        pub fn new(registry: &mut Registry) -> Self {
            let in_flight_requests: Family<QueryLabels, Gauge> = Default::default();
            registry.register(
                "in_flight_queries",
                "Current number of queries in-flight",
                in_flight_requests.clone(),
            );
            let total_requests: Family<QueryLabels, Counter> = Default::default();
            registry.register(
                "requests",
                "Total number of requests received",
                total_requests.clone(),
            );
            let request_duration: Family<QueryLabels, Histogram> =
                Family::new_with_constructor(|| {
                    Histogram::new(histogram::exponential_buckets(0.010, 2.0, 10))
                });
            registry.register(
                "request_duration_seconds",
                "Duration of seconds used to fetch all of the block information",
                request_duration.clone(),
            );
            let active_subscriptions: Gauge = Default::default();
            registry.register(
                "active_subscription",
                "Current number of active subscriptions",
                active_subscriptions.clone(),
            );
            MonitorExtension {
                in_flight_requests,
                total_requests,
                request_duration,
                active_subscriptions,
            }
        }
    }
    impl async_graphql::extensions::ExtensionFactory for MonitorExtension {
        fn create(&self) -> Arc<dyn async_graphql::extensions::Extension> { Arc::new(self.clone()) }
    }
    #[async_trait]
    impl async_graphql::extensions::Extension for MonitorExtension {
        async fn execute(
            &self,
            ctx: &async_graphql::extensions::ExtensionContext<'_>,
            operation_name: Option<&str>,
            next: async_graphql::extensions::NextExecute<'_>,
        ) -> async_graphql::Response {
            let label = QueryLabels {
                query: operation_name.unwrap_or("<none>").to_owned(),
            };
            self.in_flight_requests.get_or_create(&label).inc();
            self.total_requests.get_or_create(&label).inc();
            let start = Instant::now();
            let response = next.run(ctx, operation_name).await;
            let duration = start.elapsed();
            self.request_duration.get_or_create(&label).observe(duration.as_secs_f64());
            self.in_flight_requests.get_or_create(&label).dec();
            response
        }

        /// Called at subscribe request.
        fn subscribe<'s>(
            &self,
            ctx: &async_graphql::extensions::ExtensionContext<'_>,
            stream: stream::BoxStream<'s, async_graphql::Response>,
            next: async_graphql::extensions::NextSubscribe<'_>,
        ) -> stream::BoxStream<'s, async_graphql::Response> {
            let stream = next.run(ctx, stream);
            let wrapped_stream = WrappedStream::new(stream, self.active_subscriptions.clone());
            wrapped_stream.boxed()
        }
    }
    /// Wrapper around a stream to update metrics when it gets dropped.
    struct WrappedStream<'s> {
        inner:                stream::BoxStream<'s, async_graphql::Response>,
        active_subscriptions: Gauge,
    }
    impl<'s> WrappedStream<'s> {
        fn new(
            stream: stream::BoxStream<'s, async_graphql::Response>,
            active_subscriptions: Gauge,
        ) -> Self {
            active_subscriptions.inc();
            Self {
                inner: stream,
                active_subscriptions,
            }
        }
    }
    impl futures::stream::Stream for WrappedStream<'_> {
        type Item = async_graphql::Response;

        fn poll_next(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Self::Item>> {
            self.inner.poll_next_unpin(cx)
        }
    }
    impl std::ops::Drop for WrappedStream<'_> {
        fn drop(&mut self) { self.active_subscriptions.dec(); }
    }
}

/// All the errors that may be produced by the GraphQL API.
///
/// Note that `async_graphql` requires this to be `Clone`, as it is used as a
/// return type in queries. However, some of the underlying error types are not
/// `Clone`, so we wrap those in `Arc`s to make them `Clone`.
#[derive(Debug, thiserror::Error, Clone)]
pub enum ApiError {
    #[error("Could not find resource")]
    NotFound,
    #[error("Internal error (no NoDatabasePool): {}", .0.message)]
    NoDatabasePool(async_graphql::Error),
    #[error("Internal error (no NoServiceConfig): {}", .0.message)]
    NoServiceConfig(async_graphql::Error),
    #[error("Internal error (no MemoryState): {}", .0.message)]
    MemoryState(async_graphql::Error),
    #[error("Internal error: {0}")]
    FailedDatabaseQuery(Arc<sqlx::Error>),
    #[error("Invalid ID format: {0}")]
    InvalidIdInt(std::num::ParseIntError),
    #[error("The period cannot be converted")]
    DurationOutOfRange(Arc<Box<dyn Error + Send + Sync>>),
    #[error("The \"first\" and \"last\" parameters cannot exist at the same time")]
    QueryConnectionFirstLast,
    #[error("Internal error: {0}")]
    InternalError(String),
    #[error("Invalid integer: {0}")]
    InvalidInt(#[from] std::num::TryFromIntError),
    #[error("Invalid integer: {0}")]
    InvalidIntString(#[from] std::num::ParseIntError),
    #[error("Parse error: {0}")]
    InvalidContractVersion(#[from] InvalidContractVersionError),
    #[error("Schema in database should be a valid versioned module schema")]
    InvalidVersionedModuleSchema(#[from] VersionedSchemaError),
}

impl From<sqlx::Error> for ApiError {
    fn from(value: sqlx::Error) -> Self { ApiError::FailedDatabaseQuery(Arc::new(value)) }
}

pub type ApiResult<A> = Result<A, ApiError>;

/// Get the database pool from the context.
pub fn get_pool<'a>(ctx: &Context<'a>) -> ApiResult<&'a PgPool> {
    ctx.data::<PgPool>().map_err(ApiError::NoDatabasePool)
}

/// Get service configuration from the context.
pub fn get_config<'a>(ctx: &Context<'a>) -> ApiResult<&'a ApiServiceConfig> {
    let service_config = ctx.data::<ServiceConfig>().map_err(ApiError::NoServiceConfig)?;
    Ok(&service_config.api_config)
}

/// Get the memory state from the context.
pub fn get_memory_state<'a>(ctx: &Context<'a>) -> ApiResult<&'a MemoryState> {
    let service_config = ctx.data::<ServiceConfig>().map_err(ApiError::NoServiceConfig)?;
    Ok(&service_config.memory_state)
}

trait ConnectionCursor {
    const MIN: Self;
    const MAX: Self;
}
impl ConnectionCursor for i64 {
    const MAX: i64 = i64::MAX;
    const MIN: i64 = i64::MIN;
}
impl ConnectionCursor for usize {
    const MAX: usize = usize::MAX;
    const MIN: usize = usize::MIN;
}

struct ConnectionQuery<A> {
    from:  A,
    to:    A,
    limit: i64,
    desc:  bool,
}
impl<A> ConnectionQuery<A> {
    fn new<E>(
        first: Option<u64>,
        after: Option<String>,
        last: Option<u64>,
        before: Option<String>,
        connection_limit: u64,
    ) -> ApiResult<Self>
    where
        A: std::str::FromStr<Err = E> + ConnectionCursor,
        E: Into<ApiError>, {
        if first.is_some() && last.is_some() {
            return Err(ApiError::QueryConnectionFirstLast);
        }

        let from = if let Some(a) = after {
            a.parse::<A>().map_err(|e| e.into())?
        } else {
            A::MIN
        };

        let to = if let Some(b) = before {
            b.parse::<A>().map_err(|e| e.into())?
        } else {
            A::MAX
        };

        let limit =
            first.or(last).map_or(connection_limit, |limit| connection_limit.min(limit)) as i64;

        Ok(Self {
            from,
            to,
            limit,
            desc: last.is_some(),
        })
    }
}

#[derive(Default)]
struct BaseQuery;

#[Object]
#[allow(clippy::too_many_arguments)]
impl BaseQuery {
    async fn versions(&self) -> Versions {
        Versions {
            backend_versions: VERSION.to_string(),
        }
    }

    async fn import_state<'a>(&self, ctx: &Context<'a>) -> ApiResult<ImportState> {
        let memory_state = get_memory_state(ctx)?;

        Ok(ImportState {
            epoch_duration: TimeSpan(
                Duration::try_milliseconds(memory_state.current_epoch_duration.num_milliseconds())
                    .ok_or(ApiError::InternalError(
                        "Epoch duration in the memory state should be a valid duration in \
                         milliseconds."
                            .to_string(),
                    ))?,
            ),
        })
    }

    async fn latest_chain_parameters<'a>(
        &self,
        ctx: &Context<'a>,
    ) -> ApiResult<LatestChainParameters> {
        let memory_state = get_memory_state(ctx)?;

        Ok(LatestChainParameters {
            reward_period_length: memory_state
                .current_reward_period_length
                .reward_period_epochs()
                .epoch,
        })
    }

    async fn tokens(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Returns the first _n_ elements from the list.")] first: Option<u64>,
        #[graphql(desc = "Returns the elements in the list that come after the specified cursor.")]
        after: Option<String>,
        #[graphql(desc = "Returns the last _n_ elements from the list.")] last: Option<u64>,
        #[graphql(desc = "Returns the elements in the list that come before the specified cursor.")]
        before: Option<String>,
    ) -> ApiResult<connection::Connection<String, Token>> {
        let pool = get_pool(ctx)?;
        let config = get_config(ctx)?;

        let query = ConnectionQuery::<i64>::new(
            first,
            after,
            last,
            before,
            config.tokens_connection_limit,
        )?;

        let mut row_stream = sqlx::query_as!(
            Token,
            "SELECT * FROM (
                SELECT
                    index,
                    init_transaction_index,
                    total_supply as raw_total_supply,
                    token_id,
                    contract_index,
                    contract_sub_index,
                    token_address,
                    metadata_url
                FROM tokens
                WHERE tokens.index > $1 AND tokens.index < $2
                ORDER BY
                    (CASE WHEN $4 THEN tokens.index END) DESC,
                    (CASE WHEN NOT $4 THEN tokens.index END) ASC
                LIMIT $3
            ) AS token_data
            ORDER BY token_data.index ASC",
            query.from,
            query.to,
            query.limit,
            query.desc
        )
        .fetch(pool);

        let mut connection = connection::Connection::new(false, false);

        let mut page_max_index = None;
        while let Some(token) = row_stream.try_next().await? {
            page_max_index = Some(match page_max_index {
                None => token.index,
                Some(current_max) => max(current_max, token.index),
            });
            connection.edges.push(connection::Edge::new(token.index.to_string(), token));
        }

        if let Some(page_max_index) = page_max_index {
            let max_index = sqlx::query_scalar!(
                "
                    SELECT MAX(index) as max_index
                    FROM tokens
                "
            )
            .fetch_one(pool)
            .await?;

            if let Some(edge) = connection.edges.last() {
                connection.has_next_page = max_index.map_or(false, |db_max| db_max > page_max_index)
            }
        }

        if let Some(edge) = connection.edges.first() {
            connection.has_previous_page = edge.node.index != 0;
        }

        Ok(connection)
    }

    async fn search(&self, query: String) -> SearchResult {
        SearchResult {
            query,
        }
    }

    // bakerMetrics(period: MetricsPeriod!): BakerMetrics!
    // rewardMetrics(period: MetricsPeriod!): RewardMetrics!
    // rewardMetricsForAccount(accountId: ID! period: MetricsPeriod!):
    // RewardMetrics! poolRewardMetricsForPassiveDelegation(period:
    // MetricsPeriod!): PoolRewardMetrics!
    // poolRewardMetricsForBakerPool(bakerId: ID! period: MetricsPeriod!):
    // PoolRewardMetrics! passiveDelegation: PassiveDelegation
    // paydayStatus: PaydayStatus
    // nodeStatuses(sortField: NodeSortField! sortDirection: NodeSortDirection!
    // "Returns the first _n_ elements from the list." first: Int "Returns the
    // elements in the list that come after the specified cursor." after: String
    // "Returns the last _n_ elements from the list." last: Int "Returns the
    // elements in the list that come before the specified cursor." before: String):
    // NodeStatusesConnection nodeStatus(id: ID!): NodeStatus
}

pub struct Subscription {
    block_added:      broadcast::Receiver<Block>,
    accounts_updated: broadcast::Receiver<AccountsUpdatedSubscriptionItem>,
}

impl Subscription {
    pub fn new(retry_delay_sec: u64) -> (Self, SubscriptionContext) {
        let (block_added_sender, block_added) = broadcast::channel(100);
        let (accounts_updated_sender, accounts_updated) = broadcast::channel(100);
        (
            Subscription {
                block_added,
                accounts_updated,
            },
            SubscriptionContext {
                block_added_sender,
                accounts_updated_sender,
                retry_delay_sec,
            },
        )
    }
}

#[Subscription]
impl Subscription {
    async fn block_added(&self) -> impl Stream<Item = Result<Block, BroadcastStreamRecvError>> {
        tokio_stream::wrappers::BroadcastStream::new(self.block_added.resubscribe())
    }

    async fn accounts_updated(
        &self,
        account_address: Option<String>,
    ) -> impl Stream<Item = Result<AccountsUpdatedSubscriptionItem, BroadcastStreamRecvError>> {
        let stream =
            tokio_stream::wrappers::BroadcastStream::new(self.accounts_updated.resubscribe());

        // Apply filtering based on `account_address`.
        stream.filter_map(
            move |item: Result<AccountsUpdatedSubscriptionItem, BroadcastStreamRecvError>| {
                let address_filter = account_address.clone();
                async move {
                    match item {
                        Ok(notification) => {
                            if let Some(filter) = address_filter {
                                if notification.address == filter {
                                    // Pass on notification.
                                    Some(Ok(notification))
                                } else {
                                    // Skip if filter does not match.
                                    None
                                }
                            } else {
                                // Pass on all notification if no filter is set.
                                Some(Ok(notification))
                            }
                        }
                        // Pass on errors.
                        Err(e) => Some(Err(e)),
                    }
                }
            },
        )
    }
}

pub struct SubscriptionContext {
    block_added_sender:      broadcast::Sender<Block>,
    accounts_updated_sender: broadcast::Sender<AccountsUpdatedSubscriptionItem>,
    retry_delay_sec:         u64,
}

impl SubscriptionContext {
    const ACCOUNTS_UPDATED_CHANNEL: &'static str = "account_updated";
    const BLOCK_ADDED_CHANNEL: &'static str = "block_added";

    pub async fn listen(self, pool: PgPool, stop_signal: CancellationToken) -> anyhow::Result<()> {
        loop {
            match self.run_listener(&pool, &stop_signal).await {
                Ok(_) => {
                    info!("PgListener stopped gracefully.");
                    break; // Graceful exit, stop the loop
                }
                Err(err) => {
                    error!("PgListener encountered an error: {}. Retrying...", err);

                    // Check if the stop signal has been triggered before retrying
                    if stop_signal.is_cancelled() {
                        info!("Stop signal received. Exiting PgListener loop.");
                        break;
                    }

                    tokio::time::sleep(std::time::Duration::from_secs(self.retry_delay_sec)).await;
                }
            }
        }

        Ok(())
    }

    async fn run_listener(
        &self,
        pool: &PgPool,
        stop_signal: &CancellationToken,
    ) -> anyhow::Result<()> {
        let mut listener = sqlx::postgres::PgListener::connect_with(pool)
            .await
            .context("Failed to create a PostgreSQL listener")?;

        listener
            .listen_all([Self::BLOCK_ADDED_CHANNEL, Self::ACCOUNTS_UPDATED_CHANNEL])
            .await
            .context("Failed to listen to PostgreSQL notifications")?;

        let exit = stop_signal
            .run_until_cancelled(async move {
                loop {
                    let notification = listener.recv().await?;
                    match notification.channel() {
                        Self::BLOCK_ADDED_CHANNEL => {
                            let block_height = BlockHeight::from_str(notification.payload())
                                .context("Failed to parse payload of block added")?;
                            let block = Block::query_by_height(pool, block_height).await?;
                            self.block_added_sender.send(block)?;
                        }

                        Self::ACCOUNTS_UPDATED_CHANNEL => {
                            self.accounts_updated_sender.send(AccountsUpdatedSubscriptionItem {
                                address: notification.payload().to_string(),
                            })?;
                        }

                        unknown => {
                            anyhow::bail!("Received notification on unknown channel: {unknown}");
                        }
                    }
                }
            })
            .await;

        // Handle early exit due to stop signal or errors
        if let Some(result) = exit {
            result.context("Failed while listening on database changes")?;
        }

        Ok(())
    }
}

#[derive(Clone, Debug, SimpleObject)]
pub struct AccountsUpdatedSubscriptionItem {
    address: String,
}

#[derive(SimpleObject)]
struct LatestChainParameters {
    reward_period_length: ScalarRewardPeriodLength,
}

#[derive(SimpleObject)]
struct ImportState {
    epoch_duration: TimeSpan,
}

#[derive(SimpleObject)]
struct Versions {
    backend_versions: String,
}

/// Information about the offset pagination.
#[derive(SimpleObject)]
struct CollectionSegmentInfo {
    /// Indicates whether more items exist following the set defined by the
    /// clients arguments.
    has_next_page:     bool,
    /// Indicates whether more items exist prior the set defined by the clients
    /// arguments.
    has_previous_page: bool,
}

#[derive(SimpleObject)]
struct NodeStatus {
    // TODO: add below fields
    // peersList: [PeerReference!]!
    // nodeName: String
    // nodeId: String!
    // peerType: String!
    // uptime: UnsignedLong!
    // clientVersion: String!
    // averagePing: Float
    // peersCount: UnsignedLong!
    // bestBlock: String!
    // bestBlockHeight: UnsignedLong!
    // bestBlockBakerId: UnsignedLong
    // bestArrivedTime: DateTime
    // blockArrivePeriodEma: Float
    // blockArrivePeriodEmsd: Float
    // blockArriveLatencyEma: Float
    // blockArriveLatencyEmsd: Float
    // blockReceivePeriodEma: Float
    // blockReceivePeriodEmsd: Float
    // blockReceiveLatencyEma: Float
    // blockReceiveLatencyEmsd: Float
    // finalizedBlock: String!
    // finalizedBlockHeight: UnsignedLong!
    // finalizedTime: DateTime
    // finalizationPeriodEma: Float
    // finalizationPeriodEmsd: Float
    // packetsSent: UnsignedLong!
    // packetsReceived: UnsignedLong!
    // consensusRunning: Boolean!
    // bakingCommitteeMember: String!
    // consensusBakerId: UnsignedLong
    // finalizationCommitteeMember: Boolean!
    // transactionsPerBlockEma: Float
    // transactionsPerBlockEmsd: Float
    // bestBlockTransactionsSize: UnsignedLong
    // bestBlockTotalEncryptedAmount: UnsignedLong
    // bestBlockTotalAmount: UnsignedLong
    // bestBlockTransactionCount: UnsignedLong
    // bestBlockTransactionEnergyCost: UnsignedLong
    // bestBlockExecutionCost: UnsignedLong
    // bestBlockCentralBankAmount: UnsignedLong
    // blocksReceivedCount: UnsignedLong
    // blocksVerifiedCount: UnsignedLong
    // genesisBlock: String!
    // finalizationCount: UnsignedLong
    // finalizedBlockParent: String!
    // averageBytesPerSecondIn: Float!
    // averageBytesPerSecondOut: Float!
    id: types::ID,
}

#[derive(SimpleObject)]
struct Ranking {
    rank:  i32,
    total: i32,
}

struct SearchResult {
    query: String,
}

#[Object]
impl SearchResult {
    async fn contracts<'a>(
        &self,
        _ctx: &Context<'a>,
        #[graphql(desc = "Returns the first _n_ elements from the list.")] _first: Option<i32>,
        #[graphql(desc = "Returns the elements in the list that come after the specified cursor.")]
        _after: Option<String>,
        #[graphql(desc = "Returns the last _n_ elements from the list.")] _last: Option<i32>,
        #[graphql(desc = "Returns the elements in the list that come before the specified cursor.")]
        _before: Option<String>,
    ) -> ApiResult<connection::Connection<String, contract::Contract>> {
        todo_api!()
    }

    //    async fn modules(
    //        &self,
    //        #[graphql(desc = "Returns the first _n_ elements from the list.")]
    // _first: Option<i32>,        #[graphql(desc = "Returns the elements in the
    //     list that come after the specified cursor.")]
    //        _after: Option<String>,
    //        #[graphql(desc = "Returns the last _n_ elements from the list.")]
    // _last: Option<i32>,        #[graphql(desc = "Returns the elements in the
    // list that come before the     specified cursor.")]
    //        _before: Option<String>,
    //    ) -> ApiResult<connection::Connection<String, Module>> {
    //        todo_api!()
    //    }

    async fn blocks(
        &self,
        #[graphql(desc = "Returns the first _n_ elements from the list.")] _first: Option<i32>,
        #[graphql(desc = "Returns the elements in the list that come after the specified cursor.")]
        _after: Option<String>,
        #[graphql(desc = "Returns the last _n_ elements from the list.")] _last: Option<i32>,
        #[graphql(desc = "Returns the elements in the list that come before the specified cursor.")]
        _before: Option<String>,
    ) -> ApiResult<connection::Connection<String, Block>> {
        todo_api!()
    }

    async fn transactions(
        &self,
        #[graphql(desc = "Returns the first _n_ elements from the list.")] _first: Option<i32>,
        #[graphql(desc = "Returns the elements in the list that come after the specified cursor.")]
        _after: Option<String>,
        #[graphql(desc = "Returns the last _n_ elements from the list.")] _last: Option<i32>,
        #[graphql(desc = "Returns the elements in the list that come before the specified cursor.")]
        _before: Option<String>,
    ) -> ApiResult<connection::Connection<String, Transaction>> {
        todo_api!()
    }

    async fn tokens(
        &self,
        #[graphql(desc = "Returns the first _n_ elements from the list.")] _first: Option<i32>,
        #[graphql(desc = "Returns the elements in the list that come after the specified cursor.")]
        _after: Option<String>,
        #[graphql(desc = "Returns the last _n_ elements from the list.")] _last: Option<i32>,
        #[graphql(desc = "Returns the elements in the list that come before the specified cursor.")]
        _before: Option<String>,
    ) -> ApiResult<connection::Connection<String, token::Token>> {
        todo_api!()
    }

    async fn accounts(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Returns the first _n_ elements from the list.")] first: Option<u64>,
        #[graphql(desc = "Returns the elements in the list that come after the specified cursor.")]
        after: Option<String>,
        #[graphql(desc = "Returns the last _n_ elements from the list.")] last: Option<u64>,
        #[graphql(desc = "Returns the elements in the list that come before the specified cursor.")]
        before: Option<String>,
    ) -> ApiResult<connection::Connection<String, Account>> {
        let account_address_regex: Regex = Regex::new(r"^[1-9A-HJ-NP-Za-km-z]{1,50}$")
            .map_err(|_| ApiError::InternalError("Invalid regex".to_string()))?;
        let pool = get_pool(ctx)?;
        let query = ConnectionQuery::<i64>::new(first, after, last, before, 10)?;
        let mut connection = connection::Connection::new(false, false);
        if !account_address_regex.is_match(&self.query) {
            return Ok(connection);
        }

        if let Ok(parsed_address) =
            concordium_rust_sdk::common::types::AccountAddress::from_str(&self.query)
        {
            if let Some(account) = sqlx::query_as!(
                Account,
                "SELECT
                index,
                transaction_index,
                address,
                amount,
                delegated_stake,
                num_txs,
                delegated_restake_earnings,
                delegated_target_baker_id
            FROM accounts
            WHERE
                address = $1",
                parsed_address.to_string()
            )
            .fetch_optional(pool)
            .await?
            {
                connection.edges.push(connection::Edge::new(account.index.to_string(), account));
            }
            return Ok(connection);
        };
        let accounts = sqlx::query_as!(
            Account,
            r#"
                SELECT * FROM (SELECT
                    index,
                    transaction_index,
                    address,
                    amount,
                    delegated_stake,
                    num_txs,
                    delegated_restake_earnings,
                    delegated_target_baker_id
                FROM accounts
                WHERE
                    address LIKE $5 || '%'
                    AND index > $1
                    AND index < $2
                ORDER BY
                    (CASE WHEN $4 THEN index END) DESC,
                    (CASE WHEN NOT $4 THEN index END) ASC
                LIMIT $3
                ) ORDER BY index ASC"#,
            query.from,
            query.to,
            query.limit,
            query.desc,
            self.query
        )
        .fetch_all(pool)
        .await?;

        let mut min_index = None;
        let mut max_index = None;
        for account in accounts {
            min_index = Some(match min_index {
                None => account.index,
                Some(current_min) => min(current_min, account.index),
            });

            max_index = Some(match max_index {
                None => account.index,
                Some(current_max) => max(current_max, account.index),
            });
            connection.edges.push(connection::Edge::new(account.index.to_string(), account));
        }

        if let (Some(page_min_id), Some(page_max_id)) = (min_index, max_index) {
            let result = sqlx::query!(
                r#"
                    SELECT MAX(index) as max_id, MIN(index) as min_id
                    FROM accounts
                    WHERE
                        address LIKE $1 || '%'
                "#,
                &self.query
            )
            .fetch_one(pool)
            .await?;

            connection.has_previous_page =
                result.min_id.map_or(false, |db_min| db_min < page_min_id);
            connection.has_next_page = result.max_id.map_or(false, |db_max| db_max > page_max_id);
        }
        Ok(connection)
    }

    async fn bakers(
        &self,
        #[graphql(desc = "Returns the first _n_ elements from the list.")] _first: Option<i32>,
        #[graphql(desc = "Returns the elements in the list that come after the specified cursor.")]
        _after: Option<String>,
        #[graphql(desc = "Returns the last _n_ elements from the list.")] _last: Option<i32>,
        #[graphql(desc = "Returns the elements in the list that come before the specified cursor.")]
        _before: Option<String>,
    ) -> ApiResult<connection::Connection<String, baker::Baker>> {
        todo_api!()
    }

    async fn node_statuses(
        &self,
        #[graphql(desc = "Returns the first _n_ elements from the list.")] _first: Option<i32>,
        #[graphql(desc = "Returns the elements in the list that come after the specified cursor.")]
        _after: Option<String>,
        #[graphql(desc = "Returns the last _n_ elements from the list.")] _last: Option<i32>,
        #[graphql(desc = "Returns the elements in the list that come before the specified cursor.")]
        _before: Option<String>,
    ) -> ApiResult<connection::Connection<String, NodeStatus>> {
        todo_api!()
    }
}

#[derive(Enum, Clone, Copy, PartialEq, Eq)]
enum MetricsPeriod {
    LastHour,
    #[graphql(name = "LAST24_HOURS")]
    Last24Hours,
    #[graphql(name = "LAST7_DAYS")]
    Last7Days,
    #[graphql(name = "LAST30_DAYS")]
    Last30Days,
    #[graphql(name = "LAST_YEAR")]
    LastYear,
}

impl MetricsPeriod {
    /// The metrics period as a duration.
    fn as_duration(&self) -> Duration {
        match self {
            MetricsPeriod::LastHour => Duration::hours(1),
            MetricsPeriod::Last24Hours => Duration::hours(24),
            MetricsPeriod::Last7Days => Duration::days(7),
            MetricsPeriod::Last30Days => Duration::days(30),
            MetricsPeriod::LastYear => Duration::days(365),
        }
    }

    /// Duration used per bucket for a given metrics period.
    fn bucket_width(&self) -> Duration {
        match self {
            MetricsPeriod::LastHour => Duration::minutes(2),
            MetricsPeriod::Last24Hours => Duration::hours(1),
            MetricsPeriod::Last7Days => Duration::hours(6),
            MetricsPeriod::Last30Days => Duration::days(1),
            MetricsPeriod::LastYear => Duration::days(15),
        }
    }
}

#[derive(Enum, Clone, Copy, Display, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "account_statement_entry_type")]
pub enum AccountStatementEntryType {
    TransferIn,
    TransferOut,
    AmountDecrypted,
    AmountEncrypted,
    TransactionFee,
    FinalizationReward,
    FoundationReward,
    BakerReward,
    TransactionFeeReward,
}
