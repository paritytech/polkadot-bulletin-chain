import { ApiPromise, WsProvider } from '@polkadot/api';

const [,, endpoint, minBlocks = "1"] = process.argv;
const required = parseInt(minBlocks);

const provider = new WsProvider(endpoint);
const api = await ApiPromise.create({ provider, noInitWarn: true });

while (true) {
    const header = await api.rpc.chain.getHeader();
    const height = header.number.toNumber();
    if (height >= required) {
        console.log(`✅ Chain ready at ${endpoint} (height: ${height})`);
        await api.disconnect();
        process.exit(0);
    }
    await new Promise(r => setTimeout(r, 1000));
}

