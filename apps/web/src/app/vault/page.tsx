'use client';

import { useMemo } from 'react';
import { useVault } from '@/contexts/VaultContext';
import type { AccountType } from '@/lib/types';

export default function DashboardPage() {
  const { data, getBalanceReport } = useVault();
  const { accounts, transactions, postings } = data;

  const balanceReport = useMemo(() => getBalanceReport(), [getBalanceReport]);

  const netWorth = useMemo(() => {
    if (!balanceReport) return {};
    const totals: Record<string, number> = {};
    for (const acct of balanceReport.accounts) {
      const isAsset = acct.account.startsWith('Assets') || acct.account.startsWith('Asset');
      const isLiability = acct.account.startsWith('Liabilities') || acct.account.startsWith('Liability');
      if (isAsset || isLiability) {
        for (const [commodity, amount] of Object.entries(acct.balances)) {
          const val = parseFloat(amount);
          totals[commodity] = (totals[commodity] ?? 0) + val;
        }
      }
    }
    return totals;
  }, [balanceReport]);

  const recentTransactions = useMemo(
    () => transactions.slice(0, 5),
    [transactions],
  );

  const expenseBreakdown = useMemo(() => {
    if (!balanceReport) return [];
    return balanceReport.accounts
      .filter((a) => a.account.startsWith('Expenses') || a.account.startsWith('Expense'))
      .map((a) => ({
        account: a.account,
        total: Object.values(a.balances).reduce(
          (sum, v) => sum + Math.abs(parseFloat(v)),
          0,
        ),
      }))
      .sort((a, b) => b.total - a.total)
      .slice(0, 8);
  }, [balanceReport]);

  const accountsByType = useMemo(() => {
    const groups: Record<string, number> = {};
    for (const acct of accounts) {
      groups[acct.type] = (groups[acct.type] ?? 0) + 1;
    }
    return groups;
  }, [accounts]);

  const isEmpty = accounts.length === 0 && transactions.length === 0;

  if (isEmpty) {
    return (
      <div className="space-y-6">
        <h1 className="text-2xl font-bold">Dashboard</h1>
        <div className="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] p-8 text-center">
          <p className="text-4xl mb-3">🏦</p>
          <h2 className="text-lg font-semibold mb-2">Welcome to your vault</h2>
          <p className="text-sm text-[var(--color-text-secondary)] mb-4">
            Start by adding accounts and transactions. Your data is encrypted
            and never leaves your browser.
          </p>
          <a
            href="/vault/accounts"
            className="inline-block rounded-lg bg-[var(--color-accent)] px-4 py-2 text-sm font-semibold text-white hover:opacity-90"
          >
            Add Your First Account
          </a>
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <h1 className="text-2xl font-bold">Dashboard</h1>

      {/* Net Worth */}
      <div className="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] p-5">
        <h2 className="text-sm font-medium text-[var(--color-text-secondary)] mb-2">
          Net Worth
        </h2>
        {Object.keys(netWorth).length > 0 ? (
          <div className="space-y-1">
            {Object.entries(netWorth).map(([commodity, amount]) => (
              <p key={commodity} className="text-2xl font-bold">
                {amount.toLocaleString(undefined, {
                  minimumFractionDigits: 2,
                  maximumFractionDigits: 2,
                })}{' '}
                <span className="text-base text-[var(--color-text-secondary)]">
                  {commodity}
                </span>
              </p>
            ))}
          </div>
        ) : (
          <p className="text-2xl font-bold text-[var(--color-text-secondary)]">
            —
          </p>
        )}
      </div>

      {/* Stats */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
        <div className="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] p-4">
          <p className="text-xs text-[var(--color-text-secondary)]">Accounts</p>
          <p className="text-xl font-bold">{accounts.length}</p>
        </div>
        <div className="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] p-4">
          <p className="text-xs text-[var(--color-text-secondary)]">Transactions</p>
          <p className="text-xl font-bold">{transactions.length}</p>
        </div>
        {Object.entries(accountsByType).map(([type, count]) => (
          <div
            key={type}
            className="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] p-4"
          >
            <p className="text-xs text-[var(--color-text-secondary)] capitalize">{type}</p>
            <p className="text-xl font-bold">{count}</p>
          </div>
        ))}
      </div>

      <div className="grid md:grid-cols-2 gap-6">
        {/* Recent Transactions */}
        <div className="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] p-5">
          <h2 className="text-sm font-medium text-[var(--color-text-secondary)] mb-3">
            Recent Transactions
          </h2>
          {recentTransactions.length > 0 ? (
            <div className="space-y-2">
              {recentTransactions.map((txn) => {
                const txnPostings = postings.filter(
                  (p) => p.transaction_id === txn.id,
                );
                const amounts = txnPostings
                  .filter((p) => p.amount_quantity && parseFloat(p.amount_quantity) > 0)
                  .map(
                    (p) =>
                      `${parseFloat(p.amount_quantity!).toLocaleString(undefined, { minimumFractionDigits: 2 })} ${p.amount_commodity ?? ''}`,
                  );
                return (
                  <div
                    key={txn.id}
                    className="flex items-center justify-between py-1.5 border-b border-[var(--color-border)] last:border-0"
                  >
                    <div className="min-w-0">
                      <p className="text-sm font-medium truncate">
                        {txn.description}
                      </p>
                      <p className="text-xs text-[var(--color-text-secondary)]">
                        {txn.date}
                      </p>
                    </div>
                    <span className="text-sm font-medium ml-2 flex-shrink-0">
                      {amounts[0] ?? '—'}
                    </span>
                  </div>
                );
              })}
            </div>
          ) : (
            <p className="text-sm text-[var(--color-text-secondary)]">
              No transactions yet
            </p>
          )}
        </div>

        {/* Expense Breakdown */}
        <div className="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] p-5">
          <h2 className="text-sm font-medium text-[var(--color-text-secondary)] mb-3">
            Expense Breakdown
          </h2>
          {expenseBreakdown.length > 0 ? (
            <div className="space-y-2">
              {expenseBreakdown.map((item) => {
                const maxTotal = expenseBreakdown[0]?.total ?? 1;
                const pct = (item.total / maxTotal) * 100;
                return (
                  <div key={item.account}>
                    <div className="flex items-center justify-between text-sm mb-0.5">
                      <span className="truncate">
                        {item.account.replace(/^Expenses?:/, '')}
                      </span>
                      <span className="font-medium ml-2">
                        {item.total.toLocaleString(undefined, {
                          minimumFractionDigits: 2,
                        })}
                      </span>
                    </div>
                    <div className="h-1.5 rounded-full bg-[var(--color-border)]">
                      <div
                        className="h-1.5 rounded-full bg-[var(--color-accent)]"
                        style={{ width: `${pct}%` }}
                      />
                    </div>
                  </div>
                );
              })}
            </div>
          ) : (
            <p className="text-sm text-[var(--color-text-secondary)]">
              No expense data yet
            </p>
          )}
        </div>
      </div>
    </div>
  );
}
