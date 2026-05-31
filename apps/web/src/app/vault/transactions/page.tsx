'use client';

import { useState, useMemo } from 'react';
import { useVault } from '@/contexts/VaultContext';

export default function TransactionsPage() {
  const { data, addTransaction, deleteTransaction, saveVault } = useVault();
  const { accounts, transactions, postings } = data;

  const [search, setSearch] = useState('');
  const [showForm, setShowForm] = useState(false);
  const [formDate, setFormDate] = useState(
    new Date().toISOString().slice(0, 10),
  );
  const [formDesc, setFormDesc] = useState('');
  const [formPostings, setFormPostings] = useState([
    { accountId: '', amount: '', commodity: 'USD' },
    { accountId: '', amount: '', commodity: 'USD' },
  ]);

  const filtered = useMemo(() => {
    if (!search.trim()) return transactions;
    const q = search.toLowerCase();
    return transactions.filter(
      (t) =>
        t.description.toLowerCase().includes(q) ||
        t.date.includes(q),
    );
  }, [transactions, search]);

  const grouped = useMemo(() => {
    const groups: Record<string, typeof transactions> = {};
    for (const txn of filtered) {
      const month = txn.date.slice(0, 7);
      if (!groups[month]) groups[month] = [];
      groups[month].push(txn);
    }
    return Object.entries(groups).sort(([a], [b]) => b.localeCompare(a));
  }, [filtered]);

  const handleSubmit = async () => {
    if (!formDesc.trim() || formPostings.some((p) => !p.accountId)) return;
    addTransaction(formDate, formDesc.trim(), formPostings);
    await saveVault();
    setShowForm(false);
    setFormDesc('');
    setFormPostings([
      { accountId: '', amount: '', commodity: 'USD' },
      { accountId: '', amount: '', commodity: 'USD' },
    ]);
  };

  const handleDelete = async (id: string) => {
    deleteTransaction(id);
    await saveVault();
  };

  const getAccountName = (accountId: string) => {
    return accounts.find((a) => a.id === accountId)?.name ?? accountId;
  };

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">Transactions</h1>
        <button
          onClick={() => setShowForm(!showForm)}
          className="rounded-lg bg-[var(--color-accent)] px-4 py-2 text-sm font-semibold text-white hover:opacity-90 transition-opacity"
        >
          {showForm ? 'Cancel' : '+ Add'}
        </button>
      </div>

      {/* Add Transaction Form */}
      {showForm && (
        <div className="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] p-4 space-y-3">
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="block text-xs font-medium mb-1">Date</label>
              <input
                type="date"
                value={formDate}
                onChange={(e) => setFormDate(e.target.value)}
                className="w-full rounded border border-[var(--color-border)] bg-[var(--color-bg)] px-3 py-2 text-sm text-[var(--color-text)]"
              />
            </div>
            <div>
              <label className="block text-xs font-medium mb-1">Description</label>
              <input
                type="text"
                value={formDesc}
                onChange={(e) => setFormDesc(e.target.value)}
                placeholder="Grocery store"
                className="w-full rounded border border-[var(--color-border)] bg-[var(--color-bg)] px-3 py-2 text-sm text-[var(--color-text)] placeholder:text-[var(--color-text-secondary)]"
              />
            </div>
          </div>

          {formPostings.map((posting, i) => (
            <div key={i} className="grid grid-cols-3 gap-2">
              <select
                value={posting.accountId}
                onChange={(e) => {
                  const next = [...formPostings];
                  next[i] = { ...next[i], accountId: e.target.value };
                  setFormPostings(next);
                }}
                className="col-span-1 rounded border border-[var(--color-border)] bg-[var(--color-bg)] px-2 py-2 text-sm text-[var(--color-text)]"
              >
                <option value="">Account…</option>
                {accounts.map((a) => (
                  <option key={a.id} value={a.id}>
                    {a.name}
                  </option>
                ))}
              </select>
              <input
                type="text"
                value={posting.amount}
                onChange={(e) => {
                  const next = [...formPostings];
                  next[i] = { ...next[i], amount: e.target.value };
                  setFormPostings(next);
                }}
                placeholder="Amount"
                className="rounded border border-[var(--color-border)] bg-[var(--color-bg)] px-2 py-2 text-sm text-[var(--color-text)] placeholder:text-[var(--color-text-secondary)]"
              />
              <input
                type="text"
                value={posting.commodity}
                onChange={(e) => {
                  const next = [...formPostings];
                  next[i] = { ...next[i], commodity: e.target.value };
                  setFormPostings(next);
                }}
                className="rounded border border-[var(--color-border)] bg-[var(--color-bg)] px-2 py-2 text-sm text-[var(--color-text)]"
              />
            </div>
          ))}

          <div className="flex gap-2">
            <button
              onClick={() =>
                setFormPostings([
                  ...formPostings,
                  { accountId: '', amount: '', commodity: 'USD' },
                ])
              }
              className="text-xs text-[var(--color-accent)] hover:underline"
            >
              + Add posting
            </button>
          </div>

          <button
            onClick={handleSubmit}
            disabled={!formDesc.trim() || formPostings.some((p) => !p.accountId)}
            className="rounded-lg bg-[var(--color-accent)] px-4 py-2 text-sm font-semibold text-white hover:opacity-90 disabled:opacity-50"
          >
            Save Transaction
          </button>
        </div>
      )}

      {/* Search */}
      <input
        type="text"
        value={search}
        onChange={(e) => setSearch(e.target.value)}
        placeholder="Search transactions…"
        className="w-full rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] px-4 py-2.5 text-sm text-[var(--color-text)] placeholder:text-[var(--color-text-secondary)] focus:border-[var(--color-accent)] focus:outline-none focus:ring-1 focus:ring-[var(--color-accent)]"
      />

      {/* Transaction List */}
      {grouped.length === 0 ? (
        <div className="text-center py-12">
          <p className="text-4xl mb-3">📋</p>
          <p className="text-[var(--color-text-secondary)]">
            {search ? 'No matching transactions' : 'No transactions yet'}
          </p>
        </div>
      ) : (
        <div className="space-y-6">
          {grouped.map(([month, txns]) => (
            <div key={month}>
              <h3 className="text-xs font-semibold text-[var(--color-text-secondary)] uppercase tracking-wider mb-2">
                {month}
              </h3>
              <div className="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] divide-y divide-[var(--color-border)]">
                {txns.map((txn) => {
                  const txnPostings = postings.filter(
                    (p) => p.transaction_id === txn.id,
                  );
                  return (
                    <div
                      key={txn.id}
                      className="px-4 py-3 flex items-start justify-between gap-3"
                    >
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-2">
                          <span className="text-xs text-[var(--color-text-secondary)]">
                            {txn.date}
                          </span>
                          {txn.status === 'cleared' && (
                            <span className="text-xs text-[var(--color-accent)]">✓</span>
                          )}
                        </div>
                        <p className="text-sm font-medium truncate">
                          {txn.description}
                        </p>
                        <div className="text-xs text-[var(--color-text-secondary)] mt-0.5 space-y-0.5">
                          {txnPostings.map((p) => (
                            <div key={p.id} className="flex justify-between">
                              <span className="truncate">
                                {getAccountName(p.account_id)}
                              </span>
                              <span className="ml-2 font-mono">
                                {p.amount_quantity
                                  ? `${p.amount_quantity} ${p.amount_commodity ?? ''}`
                                  : ''}
                              </span>
                            </div>
                          ))}
                        </div>
                      </div>
                      <button
                        onClick={() => handleDelete(txn.id)}
                        className="text-xs text-[var(--color-danger)] hover:underline flex-shrink-0"
                      >
                        Delete
                      </button>
                    </div>
                  );
                })}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
