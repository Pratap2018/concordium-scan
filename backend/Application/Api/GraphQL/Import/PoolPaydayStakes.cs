﻿namespace Application.Api.GraphQL.Import;

public class PoolPaydayStakes
{
    public long PayoutBlockId { get; init; }
    public long BakerId { get; init; }
    public long BakerStake { get; init; }
    public long DelegatedStake { get; init; }
}