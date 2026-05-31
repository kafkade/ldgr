'use client';

import { useMemo } from 'react';
import { useVault } from '@/contexts/VaultContext';

export default function BudgetPage() {
  const { data, getBalanceReport } = useVault();
  const { transactions } = data;

  const now = new Date();
  const currentMonth = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, '0')}`;
  const prevDate = new Date(now.getFullYear(), now.getMonth() - 1, 1);
  const prevMonth = `${prevDate.getFullYear()}-${String(prevDate.getMonth() + 1).padStart(2, '0')}`;

  const balanceReport = useMemo(() => getBalanceReport(), [getBalanceReport]);

  const expenseCategories = useMemo(() => {
    if (!balanceReport) return [];

    return balanceReport.accounts
      .filter(
        (a) =>
          a.account.startsWith('Expenses') || a.account.startsWith('Expense'),
      )
      .map((a) => {
        const total = Object.values(a.balances).reduce(
          (sum, v) => sum + Math.abs(parseFloat(v)),
          0,
        );
        return {
          account: a.account,
          shortName: a.account.replace(/^Expenses?:/, ''),
          total,
        };
      })
      .filter((a) => a.total > 0)
      .sort((a, b) => b.total - a.total);
  }, [balanceReport]);

  const totalExpenses = useMemo(
    () => expenseCategories.reduce((s, c) => s + c.total, 0),
    [expenseCategories],
  );

  const currentMonthTxns = useMemo(
    () => transactions.filter((t) => t.date.startsWith(currentMonth)),
    [transactions, currentMonth],
  );

  const prevMonthTxns = useMemo(
    () => transactions.filter((t) => t.date.startsWith(prevMonth)),
    [transactions, prevMonth],
  );

  return (
    <div className="space-y-6">
      <h1 className="text-2xl font-bold">Budget</h1>

      {/* Month Summary */}
      <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
        <div className="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] p-4">
          <p className="text-xs text-[var(--color-text-secondary)]">
            Total Expenses
          </p>
          <p className="text-xl font-bold">
            {totalExpenses.toLocaleString(undefined, {
              minimumFractionDigits: 2,
            })}
          </p>
        </div>
        <div className="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] p-4">
          <p className="text-xs text-[var(--color-text-secondary)]">
            This Month
          </p>
          <p className="text-xl font-bold">{currentMonthTxns.length} txns</p>
        </div>
        <div className="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] p-4">
          <p className="text-xs text-[var(--color-text-secondary)]">
            Last Month
          </p>
          <p className="text-xl font-bold">{prevMonthTxns.length} txns</p>
        </div>
      </div>

      {/* Category Breakdown */}
      {expenseCategories.length === 0 ? (
        <div className="text-center py-12">
          <p className="text-4xl mb-3">💰</p>
          <p className="text-[var(--color-text-secondary)]">
            No expense data yet
          </p>
          <p className="text-xs text-[var(--color-text-secondary)] mt-2">
            Add transactions with expense accounts to see budget tracking here.
          </p>
        </div>
      ) : (
        <div className="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] p-5">
          <h2 className="text-sm font-medium text-[var(--color-text-secondary)] mb-4">
            Expense Categories
          </h2>
          <div className="space-y-4">
            {expenseCategories.map((cat) => {
              const pct =
                totalExpenses > 0
                  ? (cat.total / totalExpenses) * 100
                  : 0;
              return (
                <div key={cat.account}>
                  <div className="flex items-center justify-between mb-1">
                    <span className="text-sm font-medium">{cat.shortName}</span>
                    <div className="flex items-center gap-2">
                      <span className="text-xs text-[var(--color-text-secondary)]">
                        {pct.toFixed(1)}%
                      </span>
                      <span className="text-sm font-mono">
                        {cat.total.toLocaleString(undefined, {
                          minimumFractionDigits: 2,
                        })}
                      </span>
                    </div>
                  </div>
                  <div className="h-2.5 rounded-full bg-[var(--color-border)]">
                    <div
                      className="h-2.5 rounded-full bg-[var(--color-accent)] transition-all"
                      style={{ width: `${pct}%` }}
                    />
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}
