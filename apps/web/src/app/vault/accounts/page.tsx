'use client';

import { useState, useMemo } from 'react';
import { useVault } from '@/contexts/VaultContext';
import type { AccountType } from '@/lib/types';

const TYPE_ORDER: AccountType[] = ['asset', 'liability', 'income', 'expense', 'equity'];
const TYPE_LABELS: Record<AccountType, string> = {
  asset: 'Assets',
  liability: 'Liabilities',
  income: 'Income',
  expense: 'Expenses',
  equity: 'Equity',
};
const TYPE_ICONS: Record<AccountType, string> = {
  asset: '💵',
  liability: '💳',
  income: '📥',
  expense: '📤',
  equity: '⚖️',
};

export default function AccountsPage() {
  const { data, addAccount, saveVault, getBalanceReport } = useVault();
  const { accounts } = data;

  const [showForm, setShowForm] = useState(false);
  const [formName, setFormName] = useState('');
  const [formType, setFormType] = useState<AccountType>('asset');
  const [formCommodity, setFormCommodity] = useState('USD');

  const balanceReport = useMemo(() => getBalanceReport(), [getBalanceReport]);

  const balanceByAccount = useMemo(() => {
    const map: Record<string, Record<string, string>> = {};
    if (balanceReport) {
      for (const ab of balanceReport.accounts) {
        map[ab.account] = ab.balances;
      }
    }
    return map;
  }, [balanceReport]);

  const grouped = useMemo(() => {
    const groups: Record<AccountType, typeof accounts> = {
      asset: [],
      liability: [],
      income: [],
      expense: [],
      equity: [],
    };
    for (const acct of accounts) {
      groups[acct.type]?.push(acct);
    }
    return groups;
  }, [accounts]);

  const handleSubmit = async () => {
    if (!formName.trim()) return;
    addAccount(formName.trim(), formType, formCommodity);
    await saveVault();
    setShowForm(false);
    setFormName('');
    setFormType('asset');
  };

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">Accounts</h1>
        <button
          onClick={() => setShowForm(!showForm)}
          className="rounded-lg bg-[var(--color-accent)] px-4 py-2 text-sm font-semibold text-white hover:opacity-90"
        >
          {showForm ? 'Cancel' : '+ Add'}
        </button>
      </div>

      {/* Add Form */}
      {showForm && (
        <div className="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] p-4 space-y-3">
          <div>
            <label className="block text-xs font-medium mb-1">
              Account Name
            </label>
            <input
              type="text"
              value={formName}
              onChange={(e) => setFormName(e.target.value)}
              placeholder="Assets:Checking"
              className="w-full rounded border border-[var(--color-border)] bg-[var(--color-bg)] px-3 py-2 text-sm text-[var(--color-text)] placeholder:text-[var(--color-text-secondary)]"
            />
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="block text-xs font-medium mb-1">Type</label>
              <select
                value={formType}
                onChange={(e) => setFormType(e.target.value as AccountType)}
                className="w-full rounded border border-[var(--color-border)] bg-[var(--color-bg)] px-3 py-2 text-sm text-[var(--color-text)]"
              >
                {TYPE_ORDER.map((t) => (
                  <option key={t} value={t}>
                    {TYPE_LABELS[t]}
                  </option>
                ))}
              </select>
            </div>
            <div>
              <label className="block text-xs font-medium mb-1">Commodity</label>
              <input
                type="text"
                value={formCommodity}
                onChange={(e) => setFormCommodity(e.target.value)}
                className="w-full rounded border border-[var(--color-border)] bg-[var(--color-bg)] px-3 py-2 text-sm text-[var(--color-text)]"
              />
            </div>
          </div>
          <button
            onClick={handleSubmit}
            disabled={!formName.trim()}
            className="rounded-lg bg-[var(--color-accent)] px-4 py-2 text-sm font-semibold text-white hover:opacity-90 disabled:opacity-50"
          >
            Save Account
          </button>
        </div>
      )}

      {/* Account Groups */}
      {accounts.length === 0 ? (
        <div className="text-center py-12">
          <p className="text-4xl mb-3">🏦</p>
          <p className="text-[var(--color-text-secondary)]">No accounts yet</p>
        </div>
      ) : (
        TYPE_ORDER.filter((t) => grouped[t].length > 0).map((type) => (
          <div key={type}>
            <h2 className="text-sm font-semibold text-[var(--color-text-secondary)] uppercase tracking-wider mb-2">
              {TYPE_ICONS[type]} {TYPE_LABELS[type]}
            </h2>
            <div className="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] divide-y divide-[var(--color-border)]">
              {grouped[type].map((acct) => {
                const bals = balanceByAccount[acct.name] ?? {};
                return (
                  <div
                    key={acct.id}
                    className="px-4 py-3 flex items-center justify-between"
                  >
                    <div>
                      <p className="text-sm font-medium">{acct.name}</p>
                      <p className="text-xs text-[var(--color-text-secondary)]">
                        {acct.commodity}
                      </p>
                    </div>
                    <div className="text-right">
                      {Object.keys(bals).length > 0 ? (
                        Object.entries(bals).map(([commodity, amount]) => (
                          <p key={commodity} className="text-sm font-mono">
                            {parseFloat(amount).toLocaleString(undefined, {
                              minimumFractionDigits: 2,
                            })}{' '}
                            {commodity}
                          </p>
                        ))
                      ) : (
                        <p className="text-sm text-[var(--color-text-secondary)]">
                          0.00
                        </p>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
          </div>
        ))
      )}
    </div>
  );
}
