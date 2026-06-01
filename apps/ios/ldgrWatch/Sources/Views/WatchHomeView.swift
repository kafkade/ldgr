import SwiftUI

/// Main watch interface: navigable list of financial glances.
struct WatchHomeView: View {
    let connectivity: PhoneConnectivityManager

    var body: some View {
        if let summary = connectivity.summary {
            NavigationStack {
                List {
                    netWorthSection(summary)
                    portfolioSection(summary)
                    budgetSection(summary)
                    watchlistSection(summary)
                    lastUpdatedSection(summary)
                }
                .navigationTitle("ldgr")
            }
        } else {
            NoDataView(connectivity: connectivity)
        }
    }

    // MARK: - Sections

    private func netWorthSection(_ summary: WatchSummary) -> some View {
        Section {
            NavigationLink {
                NetWorthView(entries: summary.netWorth)
            } label: {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Net Worth")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    if let primary = summary.netWorth.first {
                        Text(primary.amount)
                            .font(.headline.monospacedDigit())
                        if summary.netWorth.count > 1 {
                            Text("\(summary.netWorth.count) currencies")
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                        }
                    } else {
                        Text("—")
                            .font(.headline)
                            .foregroundStyle(.secondary)
                    }
                }
            }
        }
    }

    private func portfolioSection(_ summary: WatchSummary) -> some View {
        Section {
            NavigationLink {
                PortfolioView(holdings: summary.portfolio)
            } label: {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Portfolio")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    Text("\(summary.portfolio.count) holdings")
                        .font(.subheadline)
                }
            }
        }
    }

    private func budgetSection(_ summary: WatchSummary) -> some View {
        Section {
            NavigationLink {
                BudgetView(budget: summary.budget)
            } label: {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Monthly Spending")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    if let budget = summary.budget {
                        Text(budget.totalSpent)
                            .font(.subheadline.monospacedDigit())
                    } else {
                        Text("—")
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                    }
                }
            }
        }
    }

    @ViewBuilder
    private func watchlistSection(_ summary: WatchSummary) -> some View {
        if !summary.watchlist.isEmpty {
            Section("Watchlist") {
                ForEach(summary.watchlist.prefix(5), id: \.symbol) { entry in
                    HStack {
                        Text(entry.symbol)
                            .font(.caption.weight(.medium))
                        Spacer()
                        if let price = entry.price {
                            Text(price)
                                .font(.caption.monospacedDigit())
                        }
                    }
                }
            }
        }
    }

    private func lastUpdatedSection(_ summary: WatchSummary) -> some View {
        Section {
            Text("Updated \(summary.lastUpdated, style: .relative) ago")
                .font(.caption2)
                .foregroundStyle(.tertiary)
                .frame(maxWidth: .infinity, alignment: .center)
                .listRowBackground(Color.clear)
        }
    }
}
