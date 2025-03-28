﻿using Concordium.Sdk.Types;
using HotChocolate;
using HotChocolate.Types;
using AccountAddress = Application.Api.GraphQL.Accounts.AccountAddress;

namespace Application.Api.GraphQL;

[InterfaceType]
public abstract class ChainParameters
{
    [GraphQLIgnore]
    public int Id { get; init; }

    public ExchangeRate EuroPerEnergy { get; init; }
    
    public ExchangeRate MicroCcdPerEuro { get; init; }

    public int AccountCreationLimit { get; init; }

    public AccountAddress FoundationAccountAddress { get; init; }
    
    private bool Equals(ChainParameters? other)
    {
        return
            other != null &&
            GetType() == other.GetType() &&
            EuroPerEnergy.Equals(other.EuroPerEnergy) &&
            MicroCcdPerEuro.Equals(other.MicroCcdPerEuro) &&
            AccountCreationLimit == other.AccountCreationLimit &&
            FoundationAccountAddress == other.FoundationAccountAddress;
    }

    public override bool Equals(object? obj)
    {
        if (ReferenceEquals(null, obj)) return false;
        if (ReferenceEquals(this, obj)) return true;
        return obj.GetType() == GetType() && Equals(obj as ChainParameters);
    }

    public override int GetHashCode()
    {
        return Id;
    }
    
    public static bool operator ==(ChainParameters? left, ChainParameters? right)
    {
        return Equals(left, right);
    }

    public static bool operator !=(ChainParameters? left, ChainParameters? right)
    {
        return !Equals(left, right);
    }
    
    internal static ChainParameters From(IChainParameters chainParameters)
    {
        return chainParameters switch
        {
            Concordium.Sdk.Types.ChainParametersV0 chainParametersV0 => ChainParametersV0.From(chainParametersV0),
            Concordium.Sdk.Types.ChainParametersV1 chainParametersV1 => ChainParametersV1.From(chainParametersV1),
            Concordium.Sdk.Types.ChainParametersV2 chainParametersV2 => ChainParametersV2.From(chainParametersV2),
            Concordium.Sdk.Types.ChainParametersV3 chainParametersV3 => ChainParametersV3.From(chainParametersV3),
            _ => throw new NotImplementedException()
        };
    }

    /// <summary>
    /// Present from protocol version 4 and above and hence from chain parameters 1 and above.
    /// </summary>
    internal static bool TryGetPoolOwnerCooldown(
        ChainParameters chainParameters,
        out ulong? poolOwnerCooldown)
    {
        switch (chainParameters)
        {
            case ChainParametersV0:
                poolOwnerCooldown = null;
                return false;
            case ChainParametersV1 chainParametersV1:
                poolOwnerCooldown = chainParametersV1.PoolOwnerCooldown;
                return true;
            case ChainParametersV2 chainParametersV2:
                poolOwnerCooldown = chainParametersV2.PoolOwnerCooldown;
                return true;
            case ChainParametersV3 chainParametersV3:
                poolOwnerCooldown = chainParametersV3.PoolOwnerCooldown;
                return true;
            default:
                throw new ArgumentOutOfRangeException(nameof(chainParameters));
        }
    }

    /// <summary>
    /// Present from protocol version 4 and above and hence from chain parameters 1 and above.
    /// </summary>
    internal static bool TryGetPassiveCommissions(
        ChainParameters chainParameters,
        out decimal? passiveFinalizationCommission,
        out decimal? passiveBakingCommission,
        out decimal? passiveTransactionCommission)
    {
        switch (chainParameters)
        {
            case ChainParametersV0:
                passiveFinalizationCommission = null;
                passiveBakingCommission = null;
                passiveTransactionCommission = null;
                return false;
            case ChainParametersV1 chainParametersV1:
                passiveFinalizationCommission = chainParametersV1.PassiveFinalizationCommission;
                passiveBakingCommission = chainParametersV1.PassiveBakingCommission;
                passiveTransactionCommission = chainParametersV1.PassiveTransactionCommission;
                return true;
            case ChainParametersV2 chainParametersV2:
                passiveFinalizationCommission = chainParametersV2.PassiveFinalizationCommission;
                passiveBakingCommission = chainParametersV2.PassiveBakingCommission;
                passiveTransactionCommission = chainParametersV2.PassiveTransactionCommission;
                return true;
            case ChainParametersV3 chainParametersV3:
                passiveFinalizationCommission = chainParametersV3.PassiveFinalizationCommission;
                passiveBakingCommission = chainParametersV3.PassiveBakingCommission;
                passiveTransactionCommission = chainParametersV3.PassiveTransactionCommission;
                return true;
            default:
                throw new ArgumentOutOfRangeException(nameof(chainParameters));
        }
    }
    
    /// <summary>
    /// Present from protocol version 4 and above and hence from chain parameters 1 and above.
    /// </summary>
    internal static bool TryGetCommissionRanges(
        ChainParameters chainParameters,
        out CommissionRange? finalizationCommissionRange,
        out CommissionRange? bakingCommissionRange,
        out CommissionRange? transactionCommissionRange)
    {
        switch (chainParameters)
        {
            case ChainParametersV0:
                finalizationCommissionRange = null;
                bakingCommissionRange = null;
                transactionCommissionRange = null;
                return false;
            case ChainParametersV1 chainParametersV1:
                finalizationCommissionRange = chainParametersV1.FinalizationCommissionRange;
                bakingCommissionRange = chainParametersV1.BakingCommissionRange;
                transactionCommissionRange = chainParametersV1.TransactionCommissionRange;
                return true;
            case ChainParametersV2 chainParametersV2:
                finalizationCommissionRange = chainParametersV2.FinalizationCommissionRange;
                bakingCommissionRange = chainParametersV2.BakingCommissionRange;
                transactionCommissionRange = chainParametersV2.TransactionCommissionRange;
                return true;
            case ChainParametersV3 chainParametersV3:
                finalizationCommissionRange = chainParametersV3.FinalizationCommissionRange;
                bakingCommissionRange = chainParametersV3.BakingCommissionRange;
                transactionCommissionRange = chainParametersV3.TransactionCommissionRange;
                return true;
            default:
                throw new ArgumentOutOfRangeException(nameof(chainParameters));
        }
    }
    
    /// <summary>
    /// Present from protocol version 4 and above and hence from chain parameters 1 and above.
    /// </summary>
    internal static bool TryGetDelegatorCooldown(
        ChainParameters chainParameters,
        out ulong? delegatorCooldown)
    {
        switch (chainParameters)
        {
            case ChainParametersV0:
                delegatorCooldown = null;
                return false;
            case ChainParametersV1 chainParametersV1:
                delegatorCooldown = chainParametersV1.DelegatorCooldown;
                return true;
            case ChainParametersV2 chainParametersV2:
                delegatorCooldown = chainParametersV2.DelegatorCooldown;
                return true;
            case ChainParametersV3 chainParametersV3:
                delegatorCooldown = chainParametersV3.DelegatorCooldown;
                return true;
            default:
                throw new ArgumentOutOfRangeException(nameof(chainParameters));
        }
    }
    
    /// <summary>
    /// Present from protocol version 4 and above and hence from chain parameters 1 and above.
    /// </summary>
    internal static bool TryGetCapitalBoundAndLeverageFactor(
        ChainParameters chainParameters,
        out decimal? capitalBound,
        out LeverageFactor? leverageFactor)
    {
        switch (chainParameters)
        {
            case ChainParametersV0:
                capitalBound = null;
                leverageFactor = null;
                return false;
            case ChainParametersV1 chainParametersV1:
                capitalBound = chainParametersV1.CapitalBound;
                leverageFactor = chainParametersV1.LeverageBound;
                return true;
            case ChainParametersV2 chainParametersV2:
                capitalBound = chainParametersV2.CapitalBound;
                leverageFactor = chainParametersV2.LeverageBound;
                return true;
            case ChainParametersV3 chainParametersV3:
                capitalBound = chainParametersV3.CapitalBound;
                leverageFactor = chainParametersV3.LeverageBound;
                return true;
            default:
                throw new ArgumentOutOfRangeException(nameof(chainParameters));
        }
    }

    /// <summary>
    /// Attempt to extract the RewardPeriodLength from ChainParameters, this will only fail for ChainParameters of blocks prior to P4.
    /// </summary>
    internal static bool TryGetRewardPeriodLength(ChainParameters chainParameters, out ulong? rewardPeriodLength) {
        switch (chainParameters) {
            case ChainParametersV0:
                rewardPeriodLength = null;
                return false;
            case ChainParametersV1 cpv1:
                rewardPeriodLength = cpv1.RewardPeriodLength;
                return true;
            case ChainParametersV2 cpv2:
                rewardPeriodLength = cpv2.RewardPeriodLength;
                return true;
            case ChainParametersV3 cpv3:
                rewardPeriodLength = cpv3.RewardPeriodLength;
                return true;
            default:
                throw new ArgumentOutOfRangeException(nameof(chainParameters));
        }
    }
    
    
}
