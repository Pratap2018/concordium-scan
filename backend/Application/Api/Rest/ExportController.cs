﻿using System.Globalization;
using System.Threading.Tasks;
using System.Text;
using Application.Api.GraphQL.EfCore;
using Concordium.Sdk.Types;
using Microsoft.AspNetCore.Mvc;
using Microsoft.EntityFrameworkCore;
using AccountAddress = Application.Api.GraphQL.Accounts.AccountAddress;

namespace Application.Api.Rest;

[ApiController]
public class ExportController : ControllerBase
{
    private readonly IDbContextFactory<GraphQlDbContext> _dbContextFactory;
    private const int MAX_DAYS = 32;

    public ExportController(IDbContextFactory<GraphQlDbContext> dbContextFactory)
    {
        _dbContextFactory = dbContextFactory;
    }

    [HttpGet]
    [Route("rest/export/statement")]
    public async Task<ActionResult> GetStatementExport(string accountAddress, DateTime? fromTime, DateTime? toTime)
    {
        await using var dbContext = await _dbContextFactory.CreateDbContextAsync();

        if (!Concordium.Sdk.Types.AccountAddress.TryParse(accountAddress, out var parsed))
        {
            return BadRequest("Invalid account format.");
        }

        var baseAddress = new AccountAddress(parsed!.GetBaseAddress().ToString());
        var account = dbContext.Accounts
            .AsNoTracking()
            .SingleOrDefault(account => account.BaseAddress == baseAddress);
        if (account == null)
        {
            return NotFound($"Account '{accountAddress}' does not exist.");
        }

        var query = dbContext.AccountStatementEntries
            .AsNoTracking()
            .Where(x => x.AccountId == account.Id);

        if (fromTime.Value.Kind != DateTimeKind.Utc)
        {
            return BadRequest("Time zone missing on 'fromTime'.");
        }

        DateTimeOffset from = fromTime ?? DateTime.Now.AddDays(-31);

        if (toTime.Value.Kind != DateTimeKind.Utc)
        {
            return BadRequest("Time zone missing on 'toTime'.");
        }

        DateTimeOffset to = toTime ?? DateTime.Now;

        if ((to - from).TotalDays > MAX_DAYS)
        {
            return BadRequest("Chosen time span exceeds max allowed days: '{MAX_DAYS}'");
        }

        query = query.Where(e => e.Timestamp >= from);
        query = query.Where(e => e.Timestamp <= to);

        var result = query.Select(x => new
        {
            x.Timestamp,
            x.EntryType,
            x.Amount,
            x.AccountBalance
        });
        var values = await result.ToListAsync();
        if (values.Count == 0)
        {
            return new NoContentResult();
        }

        var csv = new StringBuilder("Time,Amount (CCD),Balance (CCD),Label\n");
        foreach (var v in values)
        {
            csv.Append(v.Timestamp.ToString("u"));
            csv.Append(',');
            csv.Append((v.Amount / (decimal)CcdAmount.MicroCcdPerCcd).ToString(CultureInfo.InvariantCulture));
            csv.Append(',');
            csv.Append((v.AccountBalance / (decimal)CcdAmount.MicroCcdPerCcd).ToString(CultureInfo.InvariantCulture));
            csv.Append(',');
            csv.Append(v.EntryType);
            csv.Append('\n');
        }

        var firstTime = values.First().Timestamp;
        var lastTime = values.Last().Timestamp;
        return new FileContentResult(Encoding.ASCII.GetBytes(csv.ToString()), "text/csv")
        {
            FileDownloadName = $"statement-{accountAddress}_{firstTime:yyyyMMddHHmmss}Z-{lastTime:yyyyMMddHHmmss}Z.csv"
        };
    }
}
