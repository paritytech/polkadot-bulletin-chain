import { ApiPromise, WsProvider } from '@polkadot/api';

const [,, endpoint, account, expected] = process.argv;
const expectAuth = expected === 'true';

const api = await ApiPromise.create({ provider: new WsProvider(endpoint), noInitWarn: true });

for (let i = 0; i < 10; i++) {
    const auth = await api.query.transactionStorage.authorizations({ Account: account });
    if (auth.isSome === expectAuth) {
        console.log(expectAuth ? '✅ Authorized' : '✅ Not authorized (expected)');
        await api.disconnect();
        process.exit(0);
    }
    await new Promise(r => setTimeout(r, 3000));
}

console.error(`❌ Authorization mismatch: expected ${expectAuth}`);
await api.disconnect();
process.exit(1);
