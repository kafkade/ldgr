'use client';

import { useMemo } from 'react';
import { useVault } from '@/contexts/VaultContext';

export default function InvestmentsPage() {
  const { data, getBalanceReport } = useVault();
  const { accounts } = data;

  const balanceReport = useMemo(() => getBalanceReport(), [getBalanceReport]);

  const holdings = useMemo(() => {
    if (!balanceReport) return [];

    const items: Array<{
      account: string;
      commodity: string;
      quantity: number;
    }> = [];

    for (const ab of balanceReport.accounts) {
      const isAsset =
        ab.account.startsWith('Assets') || ab.account.startsWith('Asset');
      if (!isAsset) continue;

      for (const [commodity, amount] of Object.entries(ab.balances)) {
        const qty = parseFloat(amount);
        if (qty !== 0 && commodity !== 'USD' && commodity !== 'EUR' && commodity !== 'GBP') {
          items.push({
            account: ab.account,
            commodity,
            quantity: qty,
          });
        }
      }
    }

    return items.sort((a, b) => b.quantity - a.quantity);
  }, [balanceReport]);

  const allocation = useMemo(() => {
    const byCommodity: Record<string, number> = {};
    for (const h of holdings) {
      byCommodity[h.commodity] =
        (byCommodity[h.commodity] ?? 0) + Math.abs(h.quantity);
    }
    const total = Object.values(byCommodity).reduce((s, v) => s + v, 0);
    return Object.entries(byCommodity)
      .map(([commodity, quantity]) => ({
        commodity,
        quantity,
        pct: total > 0 ? (quantity / total) * 100 : 0,
      }))
      .sort((a, b) => b.quantity - a.quantity);
  }, [holdings]);

  const hasData = holdings.length > 0;

  return (
    <div className="space-y-6">
      <h1 className="text-2xl font-bold">Investments</h1>

      {!hasData ? (
        <div className="text-center py-12">
          <p className="text-4xl mb-3">📈</p>
          <p className="text-[var(--color-text-secondary)]">
            No investment holdings found
          </p>
          <p className="text-xs text-[var(--color-text-secondary)] mt-2">
            Add transactions with non-fiat commodities (stocks, ETFs, crypto)
            to see your portfolio here.
          </p>
        </div>
      ) : (
        <>
          {/* Allocation Chart */}
          <div className="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] p-5">
            <h2 className="text-sm font-medium text-[var(--color-text-secondary)] mb-3">
              Allocation
            </h2>
            <div className="space-y-2">
              {allocation.map((item) => (
                <div key={item.commodity}>
                  <div className="flex items-center justify-between text-sm mb-0.5">
                    <span className="font-medium">{item.commodity}</span>
                    <span className="text-[var(--color-text-secondary)]">
                      {item.pct.toFixed(1)}%
                    </span>
                  </div>
                  <div className="h-2 rounded-full bg-[var(--color-border)]">
                    <div
                      className="h-2 rounded-full bg-[var(--color-accent)]"
                      style={{ width: `${item.pct}%` }}
                    />
                  </div>
                </div>
              ))}
            </div>
          </div>

          {/* Holdings Table */}
          <div className="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)]">
            <div className="px-4 py-3 border-b border-[var(--color-border)]">
              <h2 className="text-sm font-medium text-[var(--color-text-secondary)]">
                Holdings
              </h2>
            </div>
            <div className="divide-y divide-[var(--color-border)]">
              {holdings.map((h, i) => (
                <div
                  key={`${h.account}-${h.commodity}-${i}`}
                  className="px-4 py-3 flex items-center justify-between"
                >
                  <div>
                    <p className="text-sm font-medium">{h.commodity}</p>
                    <p className="text-xs text-[var(--color-text-secondary)]">
                      {h.account}
                    </p>
                  </div>
                  <p className="text-sm font-mono">
                    {h.quantity.toLocaleString(undefined, {
                      minimumFractionDigits: 2,
                      maximumFractionDigits: 6,
                    })}
                  </p>
                </div>
              ))}
            </div>
          </div>
        </>
      )}
    </div>
  );
}
