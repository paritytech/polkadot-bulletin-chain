/**
 * Export all TransactionStorage authorizations and transactions from
 * Paseo Bulletin Chain to CSV files.
 *
 * Usage:
 *   node typescript/export_chain_data.js [ws_url]
 *
 * Outputs:
 *   - authorizations.csv
 *   - transactions.csv
 */

import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws';
import { bulletin } from './.papi/descriptors/dist/index.js';
import { writeFileSync } from 'node:fs';

const NODE_WS = process.argv[2] || 'wss://paseo-bulletin-rpc.polkadot.io';

function toHex(value) {
    if (typeof value === 'string') return value;
    return '0x' + [...value].map(b => b.toString(16).padStart(2, '0')).join('');
}

function escapeCsv(value) {
    const str = String(value);
    if (str.includes(',') || str.includes('"') || str.includes('\n')) {
        return '"' + str.replace(/"/g, '""') + '"';
    }
    return str;
}

function writeCsv(filename, headers, rows) {
    const lines = [headers.join(',')];
    for (const row of rows) {
        lines.push(row.map(escapeCsv).join(','));
    }
    writeFileSync(filename, lines.join('\n') + '\n');
    console.log(`Wrote ${rows.length} rows to ${filename}`);
}

async function main() {
    console.log(`Connecting to: ${NODE_WS}`);

    const papiClient = createClient(getWsProvider(NODE_WS));
    const api = papiClient.getTypedApi(bulletin);

    try {
        // --- Authorizations ---
        console.log('\nFetching all authorizations...');
        const authEntries = await api.query.TransactionStorage.Authorizations.getEntries();

        const authRows = authEntries.map(({ keyArgs, value }) => {
            const scope = keyArgs[0];
            const scopeType = scope.type;           // "Account" or "Preimage"
            const scopeValue = scopeType === 'Account'
                ? scope.value                        // SS58 address string
                : toHex(scope.value);                // content hash hex
            return [
                scopeType,
                scopeValue,
                value.extent.transactions,
                value.extent.bytes.toString(),
                value.expiration,
            ];
        });

        writeCsv('authorizations.csv', [
            'scope_type',
            'account_or_hash',
            'remaining_transactions',
            'remaining_bytes',
            'expiration_block',
        ], authRows);

        // --- Transactions ---
        console.log('\nFetching all transactions (this may take a while)...');
        const txEntries = await api.query.TransactionStorage.Transactions.getEntries();

        const txRows = [];
        for (const { keyArgs, value: infos } of txEntries) {
            const blockNumber = keyArgs[0];
            for (let i = 0; i < infos.length; i++) {
                const info = infos[i];
                txRows.push([
                    blockNumber,
                    i,
                    toHex(info.content_hash),
                    info.hashing.type,               // "Blake2b256", "Sha2_256", or "Keccak256"
                    info.cid_codec.toString(),
                    info.size,
                    info.block_chunks,
                    toHex(info.chunk_root),
                ]);
            }
        }

        writeCsv('transactions.csv', [
            'block_number',
            'index_in_block',
            'content_hash',
            'hashing_algorithm',
            'cid_codec',
            'size_bytes',
            'block_chunks',
            'chunk_root',
        ], txRows);

        // --- Summary ---
        const retentionPeriod = await api.query.TransactionStorage.RetentionPeriod.getValue();
        console.log(`\n--- Summary ---`);
        console.log(`Authorizations: ${authRows.length}`);
        console.log(`  Account: ${authRows.filter(r => r[0] === 'Account').length}`);
        console.log(`  Preimage: ${authRows.filter(r => r[0] === 'Preimage').length}`);
        console.log(`Transactions: ${txRows.length} across ${txEntries.length} blocks`);
        console.log(`Total stored bytes: ${txRows.reduce((sum, r) => sum + Number(r[5]), 0)}`);
        console.log(`Retention period: ${retentionPeriod} blocks`);
    } finally {
        papiClient.destroy();
    }
}

await main();
