#!/usr/bin/env python3
"""Generate ~1 year of synthetic daily OHLCV for bundled finance demo CSVs."""

from __future__ import annotations

import csv
import math
import random
from datetime import date, timedelta
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "crates" / "pulsar_marketlab" / "data"
BARS = 252

SERIES = {
    "SPY": (472.65, 0.00035, 0.0085),
    "QQQ": (400.10, 0.00045, 0.0110),
    "IWM": (197.55, 0.00025, 0.0105),
    "GLD": (188.85, 0.00015, 0.0065),
}


def trading_days(count: int) -> list[date]:
    days: list[date] = []
    cursor = date(2024, 1, 2)
    while len(days) < count:
        if cursor.weekday() < 5:
            days.append(cursor)
        cursor += timedelta(days=1)
    return days


def write_csv(symbol: str, start: float, drift: float, vol: float) -> None:
    rng = random.Random(sum(ord(ch) for ch in symbol))
    price = start
    path = OUT / f"{symbol}.csv"
    rows = []
    for day in trading_days(BARS):
        shock = rng.gauss(drift, vol)
        open_ = price
        close = max(1.0, price * (1.0 + shock))
        high = max(open_, close) * (1.0 + abs(rng.gauss(0.0, vol * 0.35)))
        low = min(open_, close) * (1.0 - abs(rng.gauss(0.0, vol * 0.35)))
        volume = int(5_000_000 + abs(rng.gauss(0.0, 1.0)) * 2_500_000)
        rows.append(
            [
                day.isoformat(),
                f"{open_:.2f}",
                f"{high:.2f}",
                f"{low:.2f}",
                f"{close:.2f}",
                f"{close:.2f}",
                str(volume),
            ]
        )
        price = close

    with path.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.writer(handle)
        writer.writerow(["Date", "Open", "High", "Low", "Close", "Adj Close", "Volume"])
        writer.writerows(rows)
    print(f"Wrote {path} ({len(rows)} bars)")


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    for symbol, (start, drift, vol) in SERIES.items():
        write_csv(symbol, start, drift, vol)


if __name__ == "__main__":
    main()
