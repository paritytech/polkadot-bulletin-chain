const [,, cid, expectedData, ipfsUrl] = process.argv;

const { create } = await import('ipfs-http-client');
const client = create({ url: ipfsUrl });

for (let i = 0; i < 10; i++) {
    try {
        const chunks = [];
        for await (const chunk of client.cat(cid)) chunks.push(chunk);
        const data = '0x' + Buffer.concat(chunks).toString('hex');
        
        if (data === expectedData) {
            console.log('✅ Data verified via IPFS');
            process.exit(0);
        }
        console.error(`❌ Data mismatch: got ${data}, expected ${expectedData}`);
        process.exit(1);
    } catch (e) {
        console.log(`Waiting for IPFS... (${e.message})`);
        await new Promise(r => setTimeout(r, 3000));
    }
}

console.error('❌ IPFS verification timeout');
process.exit(1);
