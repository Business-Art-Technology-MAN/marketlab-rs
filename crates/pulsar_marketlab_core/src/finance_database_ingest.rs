//! Offline JerBouma/FinanceDatabase equities CSV → `finance_database_equities.usda` writer.
//!
//! Rows are parsed from a streaming CSV reader and emitted through a buffered writer so
//! large catalogs never load fully into memory.

use std::collections::HashSet;
use std::fmt;
use std::io::{self, BufWriter, Write};

/// Lowercase a ticker and replace `.`, `-`, `/` (and other non-ident chars) with `_`.
pub fn sanitize_ticker_segment(ticker: &str) -> String {
    ticker
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| match c {
            '.' | '-' | '/' => '_',
            c if c.is_ascii_alphanumeric() => c,
            _ => '_',
        })
        .collect()
}

/// Stable universe prim leaf: `node_asset_{sanitized_ticker}`.
pub fn stable_asset_prim_leaf(ticker: &str) -> String {
    format!("node_asset_{}", sanitize_ticker_segment(ticker))
}

/// One equities.csv row mapped into schema-bound opinions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EquityCatalogRow {
    pub symbol: String,
    pub currency: String,
    pub sector: String,
    pub industry_group: String,
    pub industry: String,
    pub exchange: String,
    pub country: String,
    pub state: String,
    pub zipcode: String,
    pub market_cap_class: String,
}

/// Map FinanceDatabase `info:sector` labels into structural `inputs:*` fallbacks.
pub fn map_sector_to_inputs(sector: &str, industry_group: &str) -> (String, String) {
    let sector = sector.trim();
    let industry_group = industry_group.trim();
    let category = if sector.is_empty() {
        "Equities".to_string()
    } else {
        sector.to_string()
    };
    let sub_category = industry_group.to_string();
    (category, sub_category)
}

/// Best-effort MIC translation for common FinanceDatabase exchange tokens.
pub fn exchange_token_to_mic(exchange: &str) -> String {
    match exchange.trim().to_ascii_uppercase().as_str() {
        "NMS" | "NASDAQ" | "XNAS" => "XNAS".to_string(),
        "NYQ" | "NYSE" | "XNYS" => "XNYS".to_string(),
        "ASE" | "AMEX" | "XASE" => "XASE".to_string(),
        "BATS" | "BATY" | "EDGX" | "EDGA" => "BATS".to_string(),
        other if !other.is_empty() => other.to_string(),
        _ => String::new(),
    }
}

#[derive(Debug)]
pub enum IngestError {
    Io(io::Error),
    Csv(csv::Error),
    MissingSymbolColumn,
    EmptySymbol { line: u64 },
}

impl fmt::Display for IngestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "{err}"),
            Self::Csv(err) => write!(f, "{err}"),
            Self::MissingSymbolColumn => write!(f, "CSV is missing a symbol column"),
            Self::EmptySymbol { line } => write!(f, "empty symbol at CSV line {line}"),
        }
    }
}

impl std::error::Error for IngestError {}

impl From<io::Error> for IngestError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<csv::Error> for IngestError {
    fn from(value: csv::Error) -> Self {
        Self::Csv(value)
    }
}

fn csv_field<'a>(record: &'a csv::StringRecord, index: Option<usize>) -> &'a str {
    index
        .and_then(|idx| record.get(idx))
        .map(str::trim)
        .unwrap_or("")
}

fn resolve_column(headers: &csv::StringRecord, candidates: &[&str]) -> Option<usize> {
    for candidate in candidates {
        if let Some(idx) = headers.iter().position(|header| header == *candidate) {
            return Some(idx);
        }
    }
    None
}

fn parse_equity_record(
    record: &csv::StringRecord,
    columns: &ColumnIndices,
    line: u64,
) -> Result<Option<EquityCatalogRow>, IngestError> {
    let symbol = csv_field(record, Some(columns.symbol)).to_string();
    if symbol.is_empty() {
        return Err(IngestError::EmptySymbol { line });
    }

    Ok(Some(EquityCatalogRow {
        symbol,
        currency: csv_field(record, columns.currency).to_string(),
        sector: csv_field(record, columns.sector).to_string(),
        industry_group: csv_field(record, columns.industry_group).to_string(),
        industry: csv_field(record, columns.industry).to_string(),
        exchange: csv_field(record, columns.exchange).to_string(),
        country: csv_field(record, columns.country).to_string(),
        state: csv_field(record, columns.state).to_string(),
        zipcode: csv_field(record, columns.zipcode).to_string(),
        market_cap_class: csv_field(record, columns.market_cap).to_string(),
    }))
}

#[derive(Clone, Copy)]
struct ColumnIndices {
    symbol: usize,
    currency: Option<usize>,
    sector: Option<usize>,
    industry_group: Option<usize>,
    industry: Option<usize>,
    exchange: Option<usize>,
    country: Option<usize>,
    state: Option<usize>,
    zipcode: Option<usize>,
    market_cap: Option<usize>,
}

fn resolve_columns(headers: &csv::StringRecord) -> Result<ColumnIndices, IngestError> {
    let symbol = resolve_column(headers, &["symbol"])
        .or_else(|| if headers.len() > 0 { Some(0) } else { None })
        .ok_or(IngestError::MissingSymbolColumn)?;

    Ok(ColumnIndices {
        symbol,
        currency: resolve_column(headers, &["currency"]),
        sector: resolve_column(headers, &["sector"]),
        industry_group: resolve_column(headers, &["industry_group"]),
        industry: resolve_column(headers, &["industry"]),
        exchange: resolve_column(headers, &["exchange"]),
        country: resolve_column(headers, &["country"]),
        state: resolve_column(headers, &["state"]),
        zipcode: resolve_column(headers, &["zipcode"]),
        market_cap: resolve_column(headers, &["market_cap"]),
    })
}

fn escape_usda_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn write_token_line(out: &mut impl Write, indent: &str, name: &str, value: &str) -> io::Result<()> {
    if value.is_empty() {
        return Ok(());
    }
    writeln!(
        out,
        "{indent}token {name} = \"{}\"",
        escape_usda_string(value)
    )
}

fn write_string_line(out: &mut impl Write, indent: &str, name: &str, value: &str) -> io::Result<()> {
    if value.is_empty() {
        return Ok(());
    }
    writeln!(
        out,
        "{indent}string {name} = \"{}\"",
        escape_usda_string(value)
    )
}

fn write_financial_asset_prim(out: &mut impl Write, row: &EquityCatalogRow) -> io::Result<()> {
    let leaf = stable_asset_prim_leaf(&row.symbol);
    let (category, sub_category) = map_sector_to_inputs(&row.sector, &row.industry_group);
    let exchange_mic = exchange_token_to_mic(&row.exchange);

    writeln!(out, "        def FinancialAsset \"{leaf}\"")?;
    writeln!(out, "        {{")?;
    let indent = "            ";
    write_token_line(out, indent, "inputs:symbol", &row.symbol)?;
    write_token_line(out, indent, "inputs:asset_class", "Equity")?;
    write_token_line(out, indent, "inputs:provider", "yahoo")?;
    write_token_line(out, indent, "inputs:exchange_mic", &exchange_mic)?;
    write_string_line(out, indent, "inputs:category", &category)?;
    write_string_line(out, indent, "inputs:sub_category", &sub_category)?;
    write_string_line(out, indent, "info:sector", &row.sector)?;
    write_string_line(out, indent, "info:industry_group", &row.industry_group)?;
    write_string_line(out, indent, "info:industry", &row.industry)?;
    write_string_line(out, indent, "info:market_cap_class", &row.market_cap_class)?;
    write_string_line(out, indent, "info:currency", &row.currency)?;
    write_string_line(out, indent, "info:country", &row.country)?;
    write_string_line(out, indent, "info:state", &row.state)?;
    write_string_line(out, indent, "info:zipcode", &row.zipcode)?;
    writeln!(out, "        }}")?;
    Ok(())
}

fn write_layer_header(out: &mut impl Write) -> io::Result<()> {
    writeln!(
        out,
        r#"#usda 1.0
(
    doc = "Hierarchical catalog data compiled from JerBouma/FinanceDatabase master metrics library."
)

def Scope "MarketLab"
{{
    def Scope "Universe"
    {{"#
    )
}

fn write_layer_footer(out: &mut impl Write) -> io::Result<()> {
    writeln!(out, "    }}\n}}")
}

/// Stream-parse `equities.csv` and emit a FinanceDatabase mirror USDA layer.
pub fn ingest_equities_csv<R: io::Read, W: io::Write>(reader: R, writer: W) -> Result<usize, IngestError> {
    let mut csv_reader = csv::Reader::from_reader(reader);
    let headers = csv_reader.headers()?.clone();
    let columns = resolve_columns(&headers)?;

    let mut buf_writer = BufWriter::new(writer);
    write_layer_header(&mut buf_writer)?;

    let mut seen_leaves = HashSet::new();
    let mut ingested = 0usize;
    let mut line = 1u64;

    for record in csv_reader.records() {
        line += 1;
        let record = record?;
        let Some(row) = parse_equity_record(&record, &columns, line)? else {
            continue;
        };
        let leaf = stable_asset_prim_leaf(&row.symbol);
        if !seen_leaves.insert(leaf) {
            continue;
        }
        write_financial_asset_prim(&mut buf_writer, &row)?;
        ingested += 1;
    }

    write_layer_footer(&mut buf_writer)?;
    buf_writer.flush()?;
    Ok(ingested)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn sanitize_replaces_dot_dash_slash_with_underscore() {
        assert_eq!(sanitize_ticker_segment("BRK.B"), "brk_b");
        assert_eq!(sanitize_ticker_segment("BF-B"), "bf_b");
        assert_eq!(sanitize_ticker_segment("RDS/A"), "rds_a");
        assert_eq!(sanitize_ticker_segment("AaPl"), "aapl");
    }

    #[test]
    fn stable_leaf_matches_node_asset_pattern() {
        assert_eq!(stable_asset_prim_leaf("BRK.B"), "node_asset_brk_b");
    }

    #[test]
    fn map_sector_to_inputs_defaults_empty_sector_to_equities() {
        let (category, sub) = map_sector_to_inputs("", "Software & Services");
        assert_eq!(category, "Equities");
        assert_eq!(sub, "Software & Services");
    }

    #[test]
    fn ingest_writes_node_asset_prims_under_universe() {
        let csv = "\
symbol,currency,sector,industry_group,industry,exchange,country,state,zipcode,market_cap
AAPL,USD,Information Technology,Software & Services,Application Software,NMS,United States,California,95014,Mega-Cap
BRK.B,USD,Financials,Diversified Financials,Asset Management,NYQ,United States,Nebraska,68102,Large-Cap
";
        let mut output = Vec::new();
        let count = ingest_equities_csv(Cursor::new(csv), &mut output).expect("ingest");
        assert_eq!(count, 2);
        let usda = String::from_utf8(output).expect("utf8");
        assert!(usda.contains("def FinancialAsset \"node_asset_aapl\""));
        assert!(usda.contains("def FinancialAsset \"node_asset_brk_b\""));
        assert!(usda.contains("string info:sector = \"Information Technology\""));
        assert!(usda.contains("token inputs:symbol = \"AAPL\""));
        assert!(usda.contains("string inputs:category = \"Information Technology\""));
        assert!(usda.contains("token inputs:exchange_mic = \"XNAS\""));
    }
}
