use super::{
    baker_and_delegator_types::{CommissionRates, DelegationSummary, PaydayPoolReward},
    get_config, get_pool, ApiError, ApiResult, ApyPeriod,
};
use crate::{
    connection::{ConnectionQuery, DescendingI64},
    scalar_types::{BigInteger, Decimal},
};
use async_graphql::{connection, Context, Object};
use concordium_rust_sdk::types::AmountFraction;
use futures::TryStreamExt;
use sqlx::{postgres::types::PgInterval, types::BigDecimal};

#[derive(Default)]
pub struct QueryPassiveDelegation;

#[Object]
impl QueryPassiveDelegation {
    async fn passive_delegation<'a>(&self, ctx: &Context<'a>) -> ApiResult<PassiveDelegation> {
        let pool = get_pool(ctx)?;

        // The `passive_delegation_payday_commission_rates` table has at most one row.
        // As such taking the `MAX()` of its fields does not change the field
        // value.
        let passive_delegation = sqlx::query_as!(
            PassiveDelegation,
            "
                SELECT 
                    COUNT(*) AS delegator_count,
                    SUM(delegated_stake) AS delegated_stake,
                    MAX(payday_transaction_commission) as payday_transaction_commission,
                    MAX(payday_baking_commission) as payday_baking_commission,             
                    MAX(payday_finalization_commission) as payday_finalization_commission
                FROM accounts 
                    CROSS JOIN passive_delegation_payday_commission_rates
                WHERE delegated_target_baker_id IS NULL AND
                    delegated_stake != 0
            "
        )
        .fetch_optional(pool)
        .await?
        .ok_or(ApiError::NotFound)?;

        Ok(passive_delegation)
    }
}

struct PassiveDelegation {
    delegator_count:                Option<i64>,
    delegated_stake:                Option<BigDecimal>,
    payday_transaction_commission:  Option<i64>,
    payday_baking_commission:       Option<i64>,
    payday_finalization_commission: Option<i64>,
    //
    // Query:
    // apy7days: apy(period: LAST7_DAYS)
    // apy30days: apy(period: LAST30_DAYS)
    // Schema:
    // apy(period: ApyPeriod!): Float
}

#[Object]
impl PassiveDelegation {
    async fn pool_rewards(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Returns the first _n_ elements from the list.")] first: Option<u64>,
        #[graphql(desc = "Returns the elements in the list that come after the specified cursor.")]
        after: Option<String>,
        #[graphql(desc = "Returns the last _n_ elements from the list.")] last: Option<u64>,
        #[graphql(desc = "Returns the elements in the list that come before the specified cursor.")]
        before: Option<String>,
    ) -> ApiResult<connection::Connection<DescendingI64, PaydayPoolReward>> {
        let pool = get_pool(ctx)?;
        let config = get_config(ctx)?;
        let query = ConnectionQuery::<DescendingI64>::new(
            first,
            after,
            last,
            before,
            config.pool_rewards_connection_limit,
        )?;
        let mut row_stream = sqlx::query_as!(
            PaydayPoolReward,
            "SELECT * FROM (
                SELECT
                    payday_block_height as block_height,
                    slot_time,
                    pool_owner,
                    payday_total_transaction_rewards as total_transaction_rewards,
                    payday_delegators_transaction_rewards as delegators_transaction_rewards,
                    payday_total_baking_rewards as total_baking_rewards,
                    payday_delegators_baking_rewards as delegators_baking_rewards,
                    payday_total_finalization_rewards as total_finalization_rewards,
                    payday_delegators_finalization_rewards as delegators_finalization_rewards
                FROM bakers_payday_pool_rewards
                    JOIN blocks ON blocks.height = payday_block_height
                WHERE pool_owner_for_primary_key = -1
                    AND payday_block_height > $2 AND payday_block_height < $1
                ORDER BY
                    (CASE WHEN $4 THEN payday_block_height END) ASC,
                    (CASE WHEN NOT $4 THEN payday_block_height END) DESC
                LIMIT $3
                ) AS rewards
            ORDER BY rewards.block_height DESC",
            i64::from(query.from),
            i64::from(query.to),
            query.limit,
            query.is_last
        )
        .fetch(pool);

        let mut connection = connection::Connection::new(false, false);
        while let Some(rewards) = row_stream.try_next().await? {
            connection.edges.push(connection::Edge::new(rewards.block_height.into(), rewards));
        }

        if let (Some(edge_min_index), Some(edge_max_index)) =
            (connection.edges.last(), connection.edges.first())
        {
            let result = sqlx::query!(
                "
                    SELECT 
                        MIN(payday_block_height) as min_index,
                        MAX(payday_block_height) as max_index
                    FROM bakers_payday_pool_rewards
                    WHERE pool_owner_for_primary_key = -1
                "
            )
            .fetch_one(pool)
            .await?;

            connection.has_previous_page =
                result.max_index.map_or(false, |db_max| db_max > edge_max_index.node.block_height);
            connection.has_next_page =
                result.min_index.map_or(false, |db_min| db_min < edge_min_index.node.block_height);
        }

        Ok(connection)
    }

    // Passive delegators are sorted descending by `staked_amount`.
    async fn delegators(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Returns the first _n_ elements from the list.")] first: Option<u64>,
        #[graphql(desc = "Returns the elements in the list that come after the specified cursor.")]
        after: Option<String>,
        #[graphql(desc = "Returns the last _n_ elements from the list.")] last: Option<u64>,
        #[graphql(desc = "Returns the elements in the list that come before the specified cursor.")]
        before: Option<String>,
    ) -> ApiResult<connection::Connection<DescendingI64, DelegationSummary>> {
        let pool = get_pool(ctx)?;
        let config = get_config(ctx)?;
        let query = ConnectionQuery::<DescendingI64>::new(
            first,
            after,
            last,
            before,
            config.delegators_connection_limit,
        )?;
        let mut row_stream = sqlx::query_as!(
            DelegationSummary,
            "SELECT * FROM (
                SELECT
                    index,
                    address as account_address,
                    delegated_restake_earnings as restake_earnings,
                    delegated_stake as staked_amount
                FROM accounts
                WHERE delegated_target_baker_id IS NULL AND
                    delegated_stake != 0 AND
                    accounts.index > $2 AND accounts.index < $1
                ORDER BY
                    (CASE WHEN $4 THEN accounts.index END) ASC,
                    (CASE WHEN NOT $4 THEN accounts.index END) DESC
                LIMIT $3
            ) AS delegators
            ORDER BY delegators.staked_amount DESC",
            i64::from(query.from),
            i64::from(query.to),
            query.limit,
            query.is_last
        )
        .fetch(pool);
        let mut connection = connection::Connection::new(false, false);
        while let Some(delegator) = row_stream.try_next().await? {
            connection.edges.push(connection::Edge::new(delegator.index.into(), delegator));
        }

        if let (Some(edge_min_index), Some(edge_max_index)) =
            (connection.edges.last(), connection.edges.first())
        {
            let result = sqlx::query!(
                "
                SELECT 
                    MAX(index) as min_index,
                    MIN(index) as max_index
                FROM accounts 
                WHERE delegated_target_baker_id IS NULL
            "
            )
            .fetch_one(pool)
            .await?;

            connection.has_previous_page =
                result.max_index.map_or(false, |db_max| db_max > edge_max_index.node.index);
            connection.has_next_page =
                result.min_index.map_or(false, |db_min| db_min < edge_min_index.node.index);
        }

        Ok(connection)
    }

    async fn delegator_count(&self) -> ApiResult<i64> { Ok(self.delegator_count.unwrap_or(0i64)) }

    async fn delegated_stake(&self) -> ApiResult<BigInteger> {
        Ok(BigInteger::from(self.delegated_stake.clone().unwrap_or_default()))
    }

    /// Total passively delegated stake as a percentage of all CCDs in
    /// existence.
    async fn delegated_stake_percentage(&self, ctx: &Context<'_>) -> ApiResult<Decimal> {
        let pool = get_pool(ctx)?;

        let total_ccd_amount: i64 = sqlx::query_scalar!(
            "
                SELECT total_amount 
                FROM blocks 
                ORDER BY height DESC 
                LIMIT 1
            "
        )
        .fetch_one(pool)
        .await?;

        // Division by 0 is not possible because `total_ccd_amount` is always a
        // positive number.
        let delegated_stake_percentage: &BigDecimal =
            &(self.delegated_stake.clone().unwrap_or_default() * 100 / total_ccd_amount);

        let delegated_stake_percentage: Decimal =
            delegated_stake_percentage.try_into().map_err(|e| {
                ApiError::InternalError(format!(
                    "Can not convert `delegated_stake_percentage` to `scalar_types::Decimal`: {}",
                    e
                ))
            })?;

        Ok(delegated_stake_percentage)
    }

    async fn commission_rates(&self) -> ApiResult<CommissionRates> {
        let payday_transaction_commission = self
            .payday_transaction_commission
            .map(u32::try_from)
            .transpose()?
            .map(|c| AmountFraction::new_unchecked(c).into());
        let payday_baking_commission = self
            .payday_baking_commission
            .map(u32::try_from)
            .transpose()?
            .map(|c| AmountFraction::new_unchecked(c).into());
        let payday_finalization_commission = self
            .payday_finalization_commission
            .map(u32::try_from)
            .transpose()?
            .map(|c| AmountFraction::new_unchecked(c).into());
        Ok(CommissionRates {
            transaction_commission:  payday_transaction_commission,
            baking_commission:       payday_baking_commission,
            finalization_commission: payday_finalization_commission,
        })
    }

    async fn apy(&self, ctx: &Context<'_>, period: ApyPeriod) -> ApiResult<Option<f64>> {
        let pool = get_pool(ctx)?;
        let interval = PgInterval::try_from(period)?;
        let apy = sqlx::query_scalar!(
            r#"WITH chain_parameter AS (
                 SELECT
                      id,
                      (EXTRACT('epoch' from '1 year'::INTERVAL) * 1000)
                          / (epoch_duration * reward_period_length)
                          AS paydays_per_year
                 FROM current_chain_parameters
                 WHERE id = true
             ) SELECT
                 EXP(AVG(LN(POWER(
                     (payday_total_transaction_rewards
                         + payday_total_baking_rewards
                         + payday_total_finalization_rewards) / delegators_stake,
                     chain_parameter.paydays_per_year
                 ))))::FLOAT8
             FROM payday_passive_pool_stakes
             JOIN blocks ON blocks.height = payday_passive_pool_stakes.payday_block
             JOIN bakers_payday_pool_rewards
                 ON blocks.height = bakers_payday_pool_rewards.payday_block_height
                 -- Primary key for passive pool is (-1)
                 AND pool_owner_for_primary_key = -1
             JOIN chain_parameter ON chain_parameter.id = true
             WHERE blocks.slot_time > NOW() - $1::INTERVAL"#,
            interval
        )
        .fetch_one(pool)
        .await?;
        Ok(apy)
    }
}
