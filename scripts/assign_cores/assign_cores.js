/**
 * Script to assign extra cores to the bulletin parachain on the relay chain.
 * 
 * This script constructs and submits a sudo(batch(assign_core)) extrinsic
 * to assign multiple cores to a specified parachain.
 * 
 * Usage:
 *   node assign_cores.js <relay_endpoint> <para_id> <cores...>
 * 
 * Example:
 *   node assign_cores.js ws://localhost:9942 1006 0 1 2
 *   (assigns cores 0, 1, and 2 to parachain 1006)
 */

const { ApiPromise, WsProvider, Keyring } = require("@polkadot/api");

async function connect(endpoint) {
    const provider = new WsProvider(endpoint);
    const api = await ApiPromise.create({
        provider,
        throwOnConnect: false,
    });
    return api;
}

async function assignCores(endpoint, paraId, cores) {
    console.log(`Connecting to relay chain at: ${endpoint}`);
    console.log(`Assigning cores [${cores.join(", ")}] to parachain ${paraId}`);

    const api = await connect(endpoint);

    // Wait for the API to be ready
    await api.isReady;

    // Create keyring and add Alice (used for sudo)
    const keyring = new Keyring({ type: "sr25519" });
    const alice = keyring.addFromUri("//Alice");

    // Create assign_core calls for each core
    // Each assignment is: (CoreAssignment::Task(para_id), PartsOf57600)
    // 57600 represents a full timeslice allocation
    const assignCoreCalls = cores.map((core) => {
        return api.tx.coretime.assignCore(
            core,                              // core number
            0,                                 // begin (immediate)
            [[{ Task: paraId }, 57600]],       // assignment: [(Task(para_id), 57600)]
            null                               // end_hint: None
        );
    });

    console.log(`Created ${assignCoreCalls.length} assign_core calls`);

    // Wrap in utility.batch
    const batchCall = api.tx.utility.batch(assignCoreCalls);
    console.log("Created batch call");

    // Wrap in sudo
    const sudoCall = api.tx.sudo.sudo(batchCall);
    console.log("Created sudo call");
    console.log(`Call data (hex): ${sudoCall.method.toHex()}`);

    // Sign and submit the transaction
    console.log("Submitting transaction...");

    return new Promise((resolve, reject) => {
        sudoCall.signAndSend(alice, { nonce: -1 }, ({ status, events, dispatchError }) => {
            console.log(`Transaction status: ${status.type}`);

            if (status.isInBlock) {
                console.log(`Transaction included in block: ${status.asInBlock.toHex()}`);
            }

            if (status.isFinalized) {
                console.log(`Transaction finalized in block: ${status.asFinalized.toHex()}`);

                // Check for errors
                if (dispatchError) {
                    if (dispatchError.isModule) {
                        const decoded = api.registry.findMetaError(dispatchError.asModule);
                        const { docs, name, section } = decoded;
                        console.error(`Error: ${section}.${name}: ${docs.join(" ")}`);
                        reject(new Error(`${section}.${name}`));
                    } else {
                        console.error(`Error: ${dispatchError.toString()}`);
                        reject(new Error(dispatchError.toString()));
                    }
                    return;
                }

                // Log events
                events.forEach(({ event }) => {
                    const { section, method, data } = event;
                    console.log(`  Event: ${section}.${method}`, data.toString());
                });

                // Check for sudo success
                const sudoSuccess = events.find(({ event }) =>
                    event.section === "sudo" && event.method === "Sudid"
                );

                if (sudoSuccess) {
                    const result = sudoSuccess.event.data[0];
                    if (result.isOk) {
                        console.log("✅ Cores assigned successfully!");
                        resolve();
                    } else {
                        console.error("❌ Sudo call failed:", result.asErr.toString());
                        reject(new Error("Sudo call failed"));
                    }
                } else {
                    console.log("✅ Transaction finalized (no sudo event found, checking events above)");
                    resolve();
                }

                api.disconnect();
            }
        }).catch((err) => {
            console.error("Error submitting transaction:", err);
            reject(err);
        });
    });
}

// Parse command line arguments
const args = process.argv.slice(2);

if (args.length < 3) {
    console.log("Usage: node assign_cores.js <relay_endpoint> <para_id> <cores...>");
    console.log("");
    console.log("Arguments:");
    console.log("  relay_endpoint  WebSocket endpoint of the relay chain (e.g., ws://localhost:9942)");
    console.log("  para_id         Parachain ID to assign cores to (e.g., 1006)");
    console.log("  cores           Space-separated list of core numbers to assign");
    console.log("");
    console.log("Example:");
    console.log("  node assign_cores.js ws://localhost:9942 1006 0 1 2");
    console.log("  (assigns cores 0, 1, and 2 to parachain 1006)");
    process.exit(1);
}

const endpoint = args[0];
const paraId = parseInt(args[1], 10);
const cores = args.slice(2).map((c) => parseInt(c, 10));

if (isNaN(paraId)) {
    console.error("Error: para_id must be a number");
    process.exit(1);
}

for (const core of cores) {
    if (isNaN(core)) {
        console.error("Error: all core numbers must be integers");
        process.exit(1);
    }
}

assignCores(endpoint, paraId, cores)
    .then(() => {
        console.log("Done!");
        process.exit(0);
    })
    .catch((err) => {
        console.error("Failed to assign cores:", err.message);
        process.exit(1);
    });

