import { HopClient } from 'hop-sdk';
import { logError, logInfo } from './logger.js';
import { setupKeyringAndSigners } from './common.js';

const args = process.argv.slice(2);
const NODE_WS = args[0] || 'ws://localhost:10000';
const SEED = args[1] || '//Alice';

async function main() {
	const message = 'Hello from HOP!';
	const data = new TextEncoder().encode(message);

	const { authorizationSigner } = setupKeyringAndSigners(SEED, '//CustomSigner');

	logInfo(`Connecting to custom network`);
	const client = HopClient.connectWithAccount(
		NODE_WS,
		authorizationSigner.publicKey,
		authorizationSigner.signBytes,
		'sr25519'
	);

	try {
		logInfo(`Sending "${message}" (${data.length} bytes)…`);
		const [ticket] = await client.send(data);
		logInfo(`HOP Ticket: [${ticket.encode()}]`);

		logInfo('Claiming data back…');
		const received = await client.claim(ticket);

		const decoded = new TextDecoder().decode(received);
		logInfo(`Data decoded: ${decoded}`);

		if (decoded === message) {
			logInfo('Round-trip successful!');
		} else {
			logError(`Mismatch! Expected "${message}", got "${decoded}"`);
			process.exit(1);
		}
	} finally {
		client.destroy();
	}
}

main().catch((err) => {
	logError(err);
	process.exit(1);
});
