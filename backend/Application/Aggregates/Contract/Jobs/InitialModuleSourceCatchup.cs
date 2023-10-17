using System.Threading;
using System.Threading.Tasks;
using Application.Aggregates.Contract.Configurations;
using Application.Aggregates.Contract.Entities;
using Application.Aggregates.Contract.Observability;
using Application.Aggregates.Contract.Resilience;
using Application.Api.GraphQL.EfCore;
using Application.Observability;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.Options;
using Serilog.Context;

namespace Application.Aggregates.Contract.Jobs;

public class InitialModuleSourceCatchup : IContractJob
{
    private readonly IContractNodeClient _client;
    private readonly IDbContextFactory<GraphQlDbContext> _dbContextFactory;
    private readonly ContractHealthCheck _healthCheck;
    private readonly ILogger _logger;
    private readonly ContractAggregateOptions _contractAggregateOptions;
    private readonly ContractAggregateJobOptions _jobOptions;

    /// <summary>
    /// WARNING - Do not change this if job already executed on environment, since it will trigger rerun of job.
    /// </summary>
    private const string JobName = "InitialModuleSourceCatchup";

    public InitialModuleSourceCatchup(
        IContractNodeClient client,
        IDbContextFactory<GraphQlDbContext> dbContextFactory,
        IOptions<ContractAggregateOptions> options,
        ContractHealthCheck healthCheck)
    {
        _client = client;
        _dbContextFactory = dbContextFactory;
        _healthCheck = healthCheck;
        _logger = Log.ForContext<InitialModuleSourceCatchup>();
        _contractAggregateOptions = options.Value;
        var gotJobOptions = _contractAggregateOptions.Jobs.TryGetValue(GetUniqueIdentifier(), out var jobOptions);
        _jobOptions = gotJobOptions ? jobOptions! : new ContractAggregateJobOptions();    
    }

    private async Task<IList<string>> GetModuleReferences()
    {
        await using var context = await _dbContextFactory.CreateDbContextAsync();

        return await context.ModuleReferenceEvents
            .AsNoTracking()
            .Where(m => m.ModuleSource == null)
            .Select(m => m.ModuleReference)
            .ToListAsync();
    }

    private async ValueTask Process(string moduleReference, ulong lastFinalized, CancellationToken token)
    {
        await Policies.GetTransientPolicy(_logger, _contractAggregateOptions.RetryCount, _contractAggregateOptions.RetryDelay)
            .ExecuteAsync(async () =>
            {
                await using var context = await _dbContextFactory.CreateDbContextAsync(token);

                var module = await context.ModuleReferenceEvents
                    .SingleAsync(m => m.ModuleReference == moduleReference, cancellationToken: token);

                if (module.ModuleSource != null)
                {
                    return;
                }

                var moduleSourceInfo = await ModuleReferenceEvent.ModuleSourceInfo.Create(_client, lastFinalized, moduleReference);
                module.UpdateWithModuleSourceInfo(moduleSourceInfo);
                await context.SaveChangesAsync(token);
                
                _logger.Information($"Processed module {moduleReference}");
            });
    }
    
    public async Task StartImport(CancellationToken token)
    {
        using var _ = TraceContext.StartActivity(GetUniqueIdentifier());
        using var __ = LogContext.PushProperty("Job", GetUniqueIdentifier());
        
        try
        {
            var moduleReferences = await GetModuleReferences();
            
            _logger.Information($"Starts process {moduleReferences.Count} modules");
            
            var consensusInfo = await _client.GetConsensusInfoAsync(token);
            
            var cycle = Parallel.ForEachAsync(moduleReferences,
                new ParallelOptions
                {
                    MaxDegreeOfParallelism = _jobOptions.MaxParallelTasks
                }, (moduleRef, cancellationToken) => Process(moduleRef, consensusInfo.LastFinalizedBlockHeight, cancellationToken));
            await cycle;
            
            _logger.Information($"Done with job {GetUniqueIdentifier()}");
        }
        catch (Exception e)
        {
            _healthCheck.AddUnhealthyJobWithMessage(GetUniqueIdentifier(), "Job stopped due to exception.");
            _logger.Fatal(e, $"{GetUniqueIdentifier()} stopped due to exception.");
            throw;
        }
    }

    public string GetUniqueIdentifier() => JobName;

    public bool ShouldNodeImportAwait() => false;
}
