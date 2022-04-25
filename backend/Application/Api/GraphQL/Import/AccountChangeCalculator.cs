﻿using Application.Api.GraphQL.Accounts;
using Application.Api.GraphQL.Blocks;
using ConcordiumSdk.NodeApi.Types;

namespace Application.Api.GraphQL.Import;

public class AccountChangeCalculator
{
    private readonly IAccountLookup _accountLookup;

    public AccountChangeCalculator(IAccountLookup accountLookup)
    {
        _accountLookup = accountLookup;
    }

    public IEnumerable<Account> GetAccounts(AccountInfo[] createdAccounts, DateTimeOffset blockSlotTime)
    {
        return createdAccounts.Select(x => new Account
        {
            Id = (long)x.AccountIndex,
            CanonicalAddressString = x.AccountAddress.AsString,
            BaseAddress = new AccountAddress(x.AccountAddress.GetBaseAddress().AsString),
            Amount = x.AccountAmount.MicroCcdValue,
            CreatedAt = blockSlotTime
        });
    }

    public AccountTransactionRelation[] GetAccountTransactionRelations(TransactionPair[] transactions)
    {
        var result = Array.Empty<AccountTransactionRelation>();

        var accountTransactions = transactions
            .Select(x => new
            {
                TransactionId = x.Target.Id,
                DistinctAccountBaseAddresses = FindAccountAddresses(x.Source)
                    .Select(address => address.GetBaseAddress())
                    .Distinct()
            })
            .SelectMany(x => x.DistinctAccountBaseAddresses
                .Select(accountBaseAddress => new
                {
                    AccountBaseAddress = accountBaseAddress.AsString,
                    x.TransactionId
                }))
            .ToArray();

        if (accountTransactions.Length > 0)
        {
            var distinctBaseAddresses = accountTransactions
                .Select(x => x.AccountBaseAddress)
                .Distinct();
            var accountIdLookup = _accountLookup.GetAccountIdsFromBaseAddresses(distinctBaseAddresses);

            result = accountTransactions
                .Select(x =>
                {
                    var accountId = accountIdLookup[x.AccountBaseAddress];
                    if (accountId.HasValue)
                        return new AccountTransactionRelation
                        {
                            AccountId = accountId.Value,
                            TransactionId = x.TransactionId
                        };
                    return null;
                })
                .Where(x => x != null)
                .ToArray()!;
        }

        return result;
    }
    
    private IEnumerable<ConcordiumSdk.Types.AccountAddress> FindAccountAddresses(TransactionSummary source)
    {
        if (source.Sender != null) yield return source.Sender;
        foreach (var address in source.Result.GetAccountAddresses())
            yield return address;
    }

    public AccountUpdate[] GetAggregatedAccountUpdates(IEnumerable<AccountBalanceUpdate> balanceUpdates, AccountTransactionRelation[] transactionRelations)
    {
        var aggregatedBalanceUpdates = balanceUpdates
            .Select(x => new { BaseAddress = x.AccountAddress.GetBaseAddress().AsString, x.AmountAdjustment })
            .GroupBy(x => x.BaseAddress)
            .Select(addressGroup => new
            {
                BaseAddress = addressGroup.Key,
                AmountAdjustment = addressGroup.Aggregate(0L, (acc, item) => acc + item.AmountAdjustment)
            })
            .ToArray();

        var baseAddresses = aggregatedBalanceUpdates.Select(x => x.BaseAddress);
        var accountIdMap = _accountLookup.GetAccountIdsFromBaseAddresses(baseAddresses);
        
        var amountAdjustmentResults = aggregatedBalanceUpdates
            .Select(x =>
            {
                var accountId = accountIdMap[x.BaseAddress] ?? throw new InvalidOperationException("Attempt at updating account that does not exist!");
                return new AccountUpdate(accountId, x.AmountAdjustment, 0);
            })
            .Where(x => x.AmountAdjustment != 0);

        var transactionResults = transactionRelations
            .GroupBy(x => x.AccountId)
            .Select(x => new { AccountId = x.Key, TransactionsAdded = x.Aggregate(0, (acc, _) => ++acc) })
            .Select(x => new AccountUpdate(x.AccountId, 0, x.TransactionsAdded));

        return amountAdjustmentResults.Concat(transactionResults)
            .GroupBy(x => x.AccountId)
            .Select(group => new AccountUpdate(@group.Key,
                @group.Aggregate(0L, (acc, item) => acc + item.AmountAdjustment),
                @group.Aggregate(0, (acc, item) => acc + item.TransactionsAdded)))
            .ToArray();
    }

    public AccountReleaseScheduleItem[] GetAccountReleaseScheduleItems(IEnumerable<TransactionPair> transactions)
    {
        AccountReleaseScheduleItem[] toInsert = Array.Empty<AccountReleaseScheduleItem>();

        var result = transactions
            .Where(transaction => transaction.Source.Result is TransactionSuccessResult)
            .SelectMany(transaction =>
            {
                return ((TransactionSuccessResult)transaction.Source.Result).Events
                    .OfType<ConcordiumSdk.NodeApi.Types.TransferredWithSchedule>()
                    .SelectMany(scheduleEvent => scheduleEvent.Amount.Select((amount, ix) => new
                    {
                        AccountBaseAddress = scheduleEvent.To.GetBaseAddress().AsString,
                        TransactionId = transaction.Target.Id,
                        ScheduleIndex = ix,
                        Timestamp = amount.Timestamp,
                        Amount = Convert.ToUInt64(amount.Amount.MicroCcdValue),
                        FromAccountBaseAddress = scheduleEvent.From.GetBaseAddress().AsString
                    }));
            }).ToArray();

        if (result.Length > 0)
        {
            var distinctBaseAddresses = result
                .Select(x => x.AccountBaseAddress)
                .Concat(result.Select(x => x.FromAccountBaseAddress))
                .Distinct();
            var accountIdMap = _accountLookup.GetAccountIdsFromBaseAddresses(distinctBaseAddresses);

            toInsert = result.Select(x => new AccountReleaseScheduleItem
                {
                    AccountId = accountIdMap[x.AccountBaseAddress] ?? throw new InvalidOperationException("Account does not exist!"),
                    TransactionId = x.TransactionId,
                    Index = x.ScheduleIndex,
                    Timestamp = x.Timestamp,
                    Amount = x.Amount,
                    FromAccountId = accountIdMap[x.FromAccountBaseAddress] ?? throw new InvalidOperationException("Account does not exist!"),
                })
                .ToArray();
        }

        return toInsert;
    }

    public IEnumerable<AccountStatementEntry> GetAccountStatementEntries(
        AccountBalanceUpdate[] balanceUpdates, AccountUpdateResult[] accountUpdateResults, Block block, TransactionPair[] transactions)
    {
        var distinctBaseAddresses = balanceUpdates
            .Select(x => x.AccountAddress.GetBaseAddress().AsString)
            .Distinct();
        var accountIdMap = _accountLookup.GetAccountIdsFromBaseAddresses(distinctBaseAddresses);

        var result = balanceUpdates.Select(x => new AccountStatementEntry
            {
                AccountId = accountIdMap[x.AccountAddress.GetBaseAddress().AsString] ?? throw new InvalidOperationException("Account not found!"),
                Index = default, // Will be set by database
                Timestamp = block.BlockSlotTime,
                Amount = x.AmountAdjustment,
                EntryType = Map(x.BalanceUpdateType),
                BlockId = block.Id,
                TransactionId = GetTransactionId(x, transactions)
            })
            .ToArray();

        foreach (var accountGroup in result.GroupBy(x => x.AccountId))
        {
            var updateResult = accountUpdateResults.Single(x => x.AccountId == accountGroup.Key);
            
            var accountBalance = updateResult.AccountBalanceBeforeUpdate;
            foreach (var entry in accountGroup)
            {
                accountBalance = (ulong)((long)accountBalance + entry.Amount);
                entry.AccountBalance = accountBalance;
            }

            if (accountBalance != updateResult.AccountBalanceAfterUpdate)
                throw new InvalidOperationException("Did not end up with the expected result!");
        }
        
        return result;
    }

    private long? GetTransactionId(AccountBalanceUpdate update, TransactionPair[] transactions)
    {
        if (update.TransactionHash == null) return null;
        
        var transaction = transactions.Single(x => x.Source.Hash == update.TransactionHash!);
        return transaction.Target.Id;
    }

    private AccountStatementEntryType Map(BalanceUpdateType input)
    {
        return input switch
        {
            BalanceUpdateType.AmountDecrypted => AccountStatementEntryType.AmountDecrypted,
            BalanceUpdateType.AmountEncrypted => AccountStatementEntryType.AmountEncrypted,
            BalanceUpdateType.BakingReward => AccountStatementEntryType.BakingReward,
            BalanceUpdateType.BlockReward => AccountStatementEntryType.BlockReward,
            BalanceUpdateType.FinalizationReward => AccountStatementEntryType.FinalizationReward,
            BalanceUpdateType.MintReward => AccountStatementEntryType.MintReward,
            BalanceUpdateType.TransactionFee => AccountStatementEntryType.TransactionFee,
            BalanceUpdateType.TransferIn => AccountStatementEntryType.TransferIn,
            BalanceUpdateType.TransferOut => AccountStatementEntryType.TransferOut,
            _ => throw new NotImplementedException()
        };
    }
}