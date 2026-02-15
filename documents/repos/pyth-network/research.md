# pyth-network/research - Repository Index

## Overview

**pyth-network/research** is a Python-based research and analytics repository for the Pyth Network oracle system. It provides tools for analyzing historical price data, evaluating publisher quality, computing reward metrics, modeling oracle reliability, benchmarking against competitors, and operating dashboards for the Express Relay (PER) system. The repository connects to ClickHouse databases (Pyth Core and Lazer) to query and analyze on-chain price feed data.

## Repository Purpose

- **Publisher quality analysis:** Evaluate uptime, price deviation, conformance, and error rates for Pyth price feed publishers.
- **Reward computation:** Calculate publisher sponsorship rewards (PRP) and ranking scores.
- **Reliability modeling:** Run graphical probabilistic models (Bayesian networks) to estimate oracle aggregate reliability given publisher failure correlations.
- **Benchmark evaluation:** Compare publisher prices against external market data sources (Datascope exchange data) for data quality assessment.
- **Competitor comparison:** Compare Pyth cross-chain price feeds against competitors (Chainlink, GMX, Band, Uniswap TWAP) and centralized exchange prices (Kaiko).
- **Lazer feed analysis:** Analyze Pyth Lazer publisher uptime, price deviation, and feed reliability.
- **Aggregation research:** Implement and backtest weighted median aggregation algorithms used by Pyth.
- **Express Relay (PER) dashboards:** Streamlit dashboards for monitoring limit order and swap activity in the Express Relay system.
- **OIS staking simulations:** Model and simulate Oracle Integrity Staking reward parameter scenarios.

## Tech Stack

- **Language:** Python 3.11 (primary), with a Rust sub-project and a TypeScript sub-project
- **Data layer:** ClickHouse (Pyth Core DB + Lazer DB), configured via `config.yaml`
- **Key Python libraries:** pandas, numpy, matplotlib, plotly, streamlit, clickhouse-driver, pgmpy (Bayesian networks), pythclient, solana/solders, papermill (notebook execution)
- **Dashboards:** Streamlit (Express Relay limit + swap dashboards)
- **Notebook analysis:** Jupyter notebooks for ad-hoc analysis (BTC ETF rumors, staking simulations, data quality evaluation)

## Repository Structure

```
research/
├── README.md                          # Setup guide and CLI usage documentation
├── Dockerfile                         # Docker image for PER dashboard (Streamlit)
├── config.yaml.sample                 # ClickHouse connection config template
├── requirements.in / requirements.txt # Python dependencies
├── .python-version                    # Python 3.11
├── btc_etf_rumor_analysis.ipynb       # BTC ETF impact analysis notebook
└── pythresearch/                      # Main Python package
    ├── aggregation/                   # Pyth aggregation algorithm implementation & backtesting
    ├── data/                          # Database clients and data scripts
    │   ├── pyth_db.py                 # PythDb class — Pyth Core ClickHouse client
    │   ├── lazer_db.py                # LazerDb class — Lazer ClickHouse client
    │   └── scripts/                   # CLI scripts for data retrieval and analysis
    ├── data_quality/                  # Benchmark evaluation against external data sources
    │   ├── core/                      # Pythnet benchmark evaluation
    │   ├── lazer/                     # Lazer benchmark evaluation
    │   └── migrations/                # ClickHouse schema migrations for DQ tables
    ├── lazer/                         # Lazer-specific analysis tools
    │   ├── feed_reliability_tests.py  # Lazer feed reliability analysis CLI
    │   └── reliability/               # Uptime, deviation, and plotting for Lazer
    ├── liquidity_oracle/              # Liquidity and volume analysis (Kaiko integration)
    ├── metrics/                       # Publisher reward metrics and quality scoring
    ├── ois/                           # Oracle Integrity Staking simulations
    │   └── sim/                       # Staking parameter simulation notebooks
    ├── per/                           # Express Relay (PER) analytics
    │   ├── clickhouse/                # PER ClickHouse schema + Rust analytics connector
    │   ├── dashboard/                 # Streamlit dashboards (limit orders + swaps)
    │   ├── per_multicall/             # Solidity contracts (forge-std, OpenZeppelin submodules)
    │   └── quote-comparison/          # TypeScript quote comparison scripts (Jupiter vs ER)
    ├── ranking/                       # Publisher quality ranking queries
    ├── reliability/                   # Graphical reliability model (Bayesian network)
    ├── utils/                         # Shared utilities (colorize, progress bars)
    └── xc_analysis/                   # Cross-chain price analysis
        ├── comparison/                # Competitor oracle comparison framework
        └── consumer_survey/           # On-chain consumer protocol analysis
```

## Key Components

### Database Clients

| Class | File | Purpose |
|-------|------|---------|
| `PythDb` | `pythresearch/data/pyth_db.py` | Client for the Pyth Core ClickHouse database. Queries publisher prices, aggregate prices, uptime, error rates, confidence metrics, cross-chain data, and Kaiko exchange data. |
| `LazerDb` | `pythresearch/data/lazer_db.py` | Client for the Lazer ClickHouse database. Queries publisher uptimes (window-based, gap-based, materialized view), feed uptime, publisher price deviations, and supports uptime backfilling. |

### Data Scripts (CLI entry points)

| Script | Module | Purpose |
|--------|--------|---------|
| Download archive | `pythresearch.data.scripts.download` | Download raw Pyth account data for all products in a time range |
| Download symbol | `pythresearch.data.scripts.download_symbol` | Download aggregate price for a specific symbol |
| Conformance | `pythresearch.data.scripts.conformance` | Run conformance tests for a publisher |
| Per-publisher conformance | `pythresearch.data.scripts.per_publisher_conformance` | Conformance results for all publishers on all symbols |
| Metrics dashboard | `pythresearch.data.scripts.metrics_dashboard` | Generate dashboard-style uptime and metrics reports |
| Uptime | `pythresearch.data.scripts.uptime` | Compute publisher uptime for sponsorship rewards |
| PRP rewards | `pythresearch.data.scripts.prp_rewards` | Compute PRP (Pyth Reward Program) token rewards |
| Aggregate uptime | `pythresearch.data.scripts.aggregate_uptime` | Aggregate uptime metrics computation |
| Reward metrics | `pythresearch.metrics.plot_metrics` | Plot per-publisher quality and calibration scores |
| Ranking by symbols | `pythresearch.ranking.ranking_by_symbols` | Query publisher quality rankings by symbol |
| Ranking by publishers | `pythresearch.ranking.ranking_by_publishers` | Query publisher quality rankings by publisher |

### Lazer Analysis

| Script | Module | Purpose |
|--------|--------|---------|
| Feed reliability tests | `pythresearch.lazer.feed_reliability_tests` | Publisher uptime, feed uptime, price deviation analysis, and visual comparison for Lazer feeds |
| Benchmark evaluation | `pythresearch.data_quality.lazer.evaluate_feeds` | Compare Lazer publisher prices against Datascope exchange benchmarks (FX, metals, equities) |
| Core benchmark evaluation | `pythresearch.data_quality.core.evaluate_feeds` | Compare Pythnet publisher prices against Datascope exchange benchmarks |

### Reliability Model

| File | Purpose |
|------|---------|
| `pythresearch/reliability/empirical_reliability_model.py` | Empirical graphical reliability model using Bayesian networks (pgmpy) to estimate aggregate oracle reliability from publisher error correlations |
| `pythresearch/reliability/original_model.py` | Original/stylistic reliability model formulation |
| `pythresearch/reliability/params_empirical.py` | Parameters for the empirical model |
| `pythresearch/reliability/params_stylistic.py` | Parameters for the stylistic model |

### Cross-Chain Analysis & Competitor Comparison

| File | Purpose |
|------|---------|
| `pythresearch/xc_analysis/comparison/comparator.py` | Compare Pyth cross-chain prices against competitor oracles (Chainlink, GMX, Band, Uniswap TWAP) and CEX prices |
| `pythresearch/xc_analysis/pull_prices.py` | Pull Pyth cross-chain price data to CSV |
| `pythresearch/xc_analysis/heartbeat_analysis.py` | Analyze heartbeat/update frequency of price feeds |
| `pythresearch/xc_analysis/consumer_survey/` | Query on-chain consumer protocols using Pyth |

### Express Relay (PER)

| Component | Path | Purpose |
|-----------|------|---------|
| Limit dashboard | `pythresearch/per/dashboard/limit/app.py` | Streamlit dashboard: opportunity fulfillment, orders, searcher uptime/metrics |
| Swap dashboard | `pythresearch/per/dashboard/swap/app.py` | Streamlit dashboard: quoter comparison, quote fulfillment, searcher uptime/inventory |
| Analytics connector | `pythresearch/per/clickhouse/per-analytics-connector/` | Rust service that processes auction server bids into ClickHouse for analytics |
| Quote comparison | `pythresearch/per/quote-comparison/` | TypeScript scripts comparing Express Relay quotes against Jupiter |

### Aggregation

| File | Purpose |
|------|---------|
| `pythresearch/aggregation/aggregate.py` | Weighted percentile/median functions used in the Pyth aggregation algorithm |
| `pythresearch/aggregation/backtest.py` | Backtesting framework for aggregation algorithms |
| `pythresearch/aggregation/plot_aggregate*.py` | Visualization of aggregation scenarios |

## External Dependencies

- **Pyth Core ClickHouse:** Stores historical on-chain price data (tables: `prices`, `bdata`, `xc_price_updates`, `exchanges`, `publisher_quality_ranking`, `symbols`)
- **Lazer ClickHouse:** Stores Lazer publisher updates (tables: `publisher_updates`, `price_feeds`, `publisher_uptime_hourly`, `publisher_uptime_per_second`)
- **Data Quality ClickHouse:** Stores benchmark comparison data (tables: `datascope_*_benchmark_data`, `*_benchmark_deviation_daily`, `source_pyth_mapping_with_expiry`)
- **Datascope:** External exchange benchmark data source (FX, metals, equities, futures, US treasuries)
- **Kaiko:** Centralized exchange price data
- **Solana RPC:** Used by PER analytics connector and consumer survey scripts

## Git Submodules

- `pythresearch/per/per_multicall/lib/forge-std` — Foundry standard library
- `pythresearch/per/per_multicall/lib/openzeppelin-contracts` — OpenZeppelin contracts
- `pythresearch/per/clickhouse/per` — PER auction server (for Rust analytics connector)

## Build & Run Commands

```bash
# Install dependencies
pip install pip-tools
pip-sync  # or: pip install -r requirements.txt

# Configure database access
cp config.yaml.sample config.yaml
# Edit config.yaml with ClickHouse credentials

# Run data scripts (examples)
python3 -m pythresearch.data.scripts.download "2021-12-10 00:00:00" "2021-12-10 01:00:00" output/
python3 -m pythresearch.metrics.plot_metrics BTC/USD "2021-12-18 00:00:00" "2021-12-18 01:00:00"
python3 -m pythresearch.data.scripts.prp_rewards "2024-03-15 00:00:00" "2024-04-15 00:00:00" --cluster pythnet

# Lazer reliability tests
python3 -m pythresearch.lazer.feed_reliability_tests "2025-03-18 00:00:00" "2025-03-19 00:00:00"

# Lazer benchmark evaluation
python3 -m pythresearch.data_quality.lazer.evaluate_feeds --csv price_id_list.csv

# Reliability model
python3 -m pythresearch.reliability.empirical_reliability_model -sd "2022-03-22 00:00:00" -ed "2022-03-23 00:00:00" -c mainnet-beta -s Crypto.FTM/USD

# Run PER limit dashboard (also via Docker)
python3 -m streamlit run pythresearch/per/dashboard/limit/app.py --server.port 8501
docker build -t per-dashboard . && docker run -p 8501:8501 per-dashboard

# PER analytics connector (Rust)
cd pythresearch/per/clickhouse/per-analytics-connector
cargo run --release -- --config config.yaml pull

# Quote comparison (TypeScript)
cd pythresearch/per/quote-comparison
npm install
npm run continuous-comparison -- --trader <TRADER_PUBKEY>
```

## When to Use This Repository

Use this repository when the task involves:

- **Analyzing Pyth publisher performance** (uptime, deviation, conformance, error rates, ranking)
- **Computing publisher rewards** (PRP rewards, sponsorship rewards, quality scores)
- **Evaluating data quality** against external benchmarks (Datascope FX/equities/futures/metals)
- **Modeling oracle reliability** (Bayesian network models for aggregate uptime/accuracy)
- **Comparing Pyth against competitors** (Chainlink, GMX, Band, Uniswap TWAP)
- **Analyzing Lazer feed reliability** (publisher uptime, price deviation, feed readiness)
- **Operating Express Relay dashboards** (limit order and swap analytics)
- **Researching aggregation algorithms** (weighted median, backtesting)
- **Running staking simulations** (OIS parameter analysis)
- **Querying Pyth historical price data** from ClickHouse

Do NOT use this repository for:

- Building or deploying Pyth on-chain programs or smart contracts
- Running Pyth validator/publisher nodes
- Frontend/UI development (except the Streamlit dashboards contained here)
- Infrastructure or cluster management