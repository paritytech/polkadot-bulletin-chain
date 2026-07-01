// Fetch a chain's spec from a live relay/solo RPC node via `sync_state_genSyncSpec`.
// The result carries a `lightSyncState` checkpoint, so smoldot warp-syncs from it
// instead of from genesis — ideal as the relay chainspec (arg 1) for the smoldot
// parachain tests. Parachains don't expose this method (they derive finality from
// the relay), so their spec still comes from the chainspecs repo.
//
// Usage: node fetch_sync_spec.js <relay_ws_url> > relay.json

const url = process.argv[2];
if (!url) {
    console.error('Usage: node fetch_sync_spec.js <relay_ws_url> > relay.json');
    process.exit(1);
}

const ws = new WebSocket(url);
const timer = setTimeout(() => {
    console.error(`Timed out fetching sync spec from ${url}`);
    process.exit(1);
}, 30000);

ws.addEventListener('open', () => {
    ws.send(JSON.stringify({ id: 1, jsonrpc: '2.0', method: 'sync_state_genSyncSpec', params: [true] }));
});

ws.addEventListener('message', (event) => {
    clearTimeout(timer);
    const msg = JSON.parse(event.data);
    if (msg.error) {
        console.error(`sync_state_genSyncSpec failed on ${url}: ${JSON.stringify(msg.error)}`);
        process.exit(1);
    }
    process.stdout.write(JSON.stringify(msg.result));
    ws.close();
    process.exit(0);
});

ws.addEventListener('error', () => {
    console.error(`WebSocket error connecting to ${url}`);
    process.exit(1);
});
