use super::{
    get_config, get_pool, transaction::Transaction, Account, ApiError, ApiResult,
    CollectionSegmentInfo,
};
use crate::{
    address::ContractIndex,
    graphql_api::todo_api,
    scalar_types::{BigInteger, TransactionIndex},
};
use async_graphql::{ComplexObject, Context, Object, SimpleObject};

#[derive(Default)]
pub struct QueryToken;

#[Object]
impl QueryToken {
    async fn token<'a>(
        &self,
        ctx: &Context<'a>,
        contract_index: ContractIndex,
        contract_sub_index: ContractIndex,
        token_id: String,
    ) -> ApiResult<Token> {
        let pool = get_pool(ctx)?;

        let token = sqlx::query_as!(
            Token,
            "SELECT
                total_supply as raw_total_supply,
                token_id,
                contract_index,
                contract_sub_index,
                token_address,
                metadata_url,
                init_transaction_index
            FROM tokens
            WHERE tokens.contract_index = $1 AND tokens.contract_sub_index = $2 AND \
             tokens.token_id = $3",
            contract_index.0 as i64,
            contract_sub_index.0 as i64,
            token_id
        )
        .fetch_optional(pool)
        .await?
        .ok_or(ApiError::NotFound)?;

        Ok(token)
    }
}

#[derive(SimpleObject)]
#[graphql(complex)]
pub struct Token {
    #[graphql(skip)]
    pub init_transaction_index: TransactionIndex,
    pub contract_index:         i64,
    pub contract_sub_index:     i64,
    pub token_id:               String,
    pub metadata_url:           Option<String>,
    #[graphql(skip)]
    pub raw_total_supply:       bigdecimal::BigDecimal,
    pub token_address:          String,
    // TODO tokenEvents(skip: Int take: Int): TokenEventsCollectionSegment
}

#[ComplexObject]
impl Token {
    async fn initial_transaction(&self, ctx: &Context<'_>) -> ApiResult<Transaction> {
        Transaction::query_by_index(get_pool(ctx)?, self.init_transaction_index).await?.ok_or(
            ApiError::InternalError("Token: No transaction at init_transaction_index".to_string()),
        )
    }

    async fn total_supply(&self, ctx: &Context<'_>) -> ApiResult<BigInteger> {
        Ok(BigInteger::from(self.raw_total_supply.clone()))
    }

    async fn contract_address_formatted(&self, ctx: &Context<'_>) -> ApiResult<String> {
        Ok(format!("<{},{}>", self.contract_index, self.contract_sub_index))
    }

    async fn accounts(
        &self,
        ctx: &Context<'_>,
        skip: Option<u64>,
        take: Option<u64>,
    ) -> ApiResult<AccountsCollectionSegment> {
        let pool = get_pool(ctx)?;
        let config = get_config(ctx)?;
        let min_index = i64::try_from(skip.unwrap_or(0))?;
        let limit =
            i64::try_from(take.map_or(config.token_holder_addresses_collection_limit, |t| {
                config.token_holder_addresses_collection_limit.min(t)
            }))?;

        // Tokens with 0 balance are filtered out. We still display tokens with a
        // negative balance (buggy cis2 smart contract) to help smart contract
        // developers to debug their smart contracts.
        // Excluding tokens with 0 balances does not scale well for smart contracts
        // with a large number of account token holdings currently. We might need to
        // introduce a specific index to optimize this query in the future.
        let mut items = sqlx::query_as!(
            AccountToken,
            "WITH filtered_tokens AS (
                SELECT
                    token_id,
                    contract_index,
                    contract_sub_index,
                    balance AS raw_balance,
                    account_index AS account_id,
                    ROW_NUMBER() OVER (ORDER BY account_tokens.index) AS row_num
                FROM account_tokens
                JOIN tokens
                    ON tokens.contract_index = $1
                    AND tokens.contract_sub_index = $2
                    AND tokens.token_id = $3
                    AND tokens.index = account_tokens.token_index
                WHERE account_tokens.balance != 0
            )
            SELECT
                token_id,
                contract_index,
                contract_sub_index,
                raw_balance,
                account_id
            FROM filtered_tokens
            WHERE row_num > $4
            LIMIT $5
        ",
            self.contract_index,
            self.contract_sub_index,
            self.token_id,
            min_index,
            limit + 1
        )
        .fetch_all(pool)
        .await?;

        // Determine if there is a next page by checking if we got more than `limit`
        // rows.
        let has_next_page = items.len() > limit as usize;
        // If there is a next page, remove the extra row used for pagination detection.
        if has_next_page {
            items.pop();
        }
        let has_previous_page = min_index > 0;

        // Tokens with 0 balance are filtered out. We still display tokens with a
        // negative balance (buggy cis2 smart contract) to help smart contract
        // developers to debug their smart contracts.
        // This counting approach below does not scale well for smart contracts
        // with a large number of account token holdings currently, since a large
        // number of rows would be traversed. This might have to be improved in the
        // future by indexing more.
        let total_count: i32 = sqlx::query_scalar!(
            "SELECT
                COUNT(*)
            FROM account_tokens
                JOIN tokens 
                ON tokens.contract_index = $1 
                AND tokens.contract_sub_index = $2 
                AND tokens.token_id = $3
            WHERE tokens.index = account_tokens.token_index 
                AND account_tokens.balance != 0",
            self.contract_index,
            self.contract_sub_index,
            self.token_id,
        )
        .fetch_one(pool)
        .await?
        .unwrap_or(0)
        .try_into()?;

        Ok(AccountsCollectionSegment {
            page_info: CollectionSegmentInfo {
                has_next_page,
                has_previous_page,
            },
            total_count,
            items,
        })
    }
}

/// A segment of a collection.
#[derive(SimpleObject)]
pub struct TokensCollectionSegment {
    /// Information to aid in pagination.
    pub page_info:   CollectionSegmentInfo,
    /// A flattened list of the items.
    pub items:       Vec<Token>,
    pub total_count: i32,
}

/// A segment of a collection.
#[derive(SimpleObject)]
pub struct AccountsCollectionSegment {
    /// Information to aid in pagination.
    pub page_info:   CollectionSegmentInfo,
    /// A flattened list of the items.
    pub items:       Vec<AccountToken>,
    pub total_count: i32,
}
#[derive(SimpleObject)]
#[graphql(complex)]
pub struct AccountToken {
    pub token_id:           String,
    pub contract_index:     i64,
    pub contract_sub_index: i64,
    #[graphql(skip)]
    pub raw_balance:        bigdecimal::BigDecimal,
    pub account_id:         i64,
}
#[ComplexObject]
impl AccountToken {
    async fn token<'a>(&self, ctx: &Context<'a>) -> ApiResult<Token> { todo_api!() }

    async fn account<'a>(&self, ctx: &Context<'a>) -> ApiResult<Account> {
        Account::query_by_index(get_pool(ctx)?, self.account_id).await?.ok_or(ApiError::NotFound)
    }

    async fn balance(&self, ctx: &Context<'_>) -> ApiResult<BigInteger> {
        Ok(BigInteger::from(self.raw_balance.clone()))
    }
}
