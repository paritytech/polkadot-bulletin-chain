import { HopClient } from 'hop-sdk';
import { DEFAULT_IPFS_GATEWAY_URL, setupKeyringAndSigners, waitForBlockProduction } from './common';
import { logInfo } from './logger';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { createClient as createPapiClient } from 'polkadot-api';
import { bulletin } from './.papi/descriptors/dist/index.js';
import { cidFromBytes } from './cid_dag_metadata.js';
import { authorizeAccount, TX_MODE_FINALIZED_BLOCK } from './api.js';

/**
 * 1. Setup authorized account (to the signer of `pallet_hop_promotion::promote` call)
 * 2. Upload data via HOP rpc
 * 3. Use authorized account to sign the payload
 * 4. Use the same / different account to call `pallet_hop_promotion::promote` (whatever signed/unsigned/sudo)
 */

// Command line arguments: [ws_url] [seed] [ipfs_api_url]
const args = process.argv.slice(2);
const NODE_WS = args[0] || 'ws://localhost:10000';
const SEED = args[1] || '//Alice';
const HTTP_IPFS_API = args[2] || DEFAULT_IPFS_GATEWAY_URL;

async function main() {
	await cryptoWaitReady();

	logHeader('AUTHORIZE AND STORE TEST (WebSocket)');
	logConnection(NODE_WS, SEED, HTTP_IPFS_API);

	// Signers
	const { authorizationSigner, whoSigner, whoAddress } = setupKeyringAndSigners(SEED, '//CustomSigner');

	// Data to store
	const message = 'Hello from HOP!';
	const data = new TextEncoder().encode(message);
	const expectedCid = await cidFromBytes(dataToStore);

	// Create PAPI client
	const papiClient = createPapiClient(getWsProvider(NODE_WS));
	const bulletinAPI = papiClient.getTypedApi(bulletin);
	await waitForBlockProduction(bulletinAPI);

	// Create HOP client
	logInfo(`Connecting to local`);
	const hopClient = HopClient.connectWithAccount(
		NODE_WS,
		authorizationSigner.publicKey,
		authorizationSigner,
		'sr25519'
	);

	try {
		logInfo("Authorizing signer...");
		await authorizeAccount(
			bulletinAPI,
			authorizationSigner,
			whoAddress,
			100, // tx quantity
			BigInt(100 * 1024 * 1024), // 100 MiB
			TX_MODE_FINALIZED_BLOCK,
		);

		logInfo(`Sending "${message}" (${data.length} bytes)…`);
		const [ticket] = await hopClient.send(data);

		logInfo(`Generating proof for HOP promotion...`)
		const proof = hopClient.generateProof(data);


	} finally {
		hopClient.destroy();
	}
}

main().catch((err) => {
	console.error(err);
	process.exit(1);
});