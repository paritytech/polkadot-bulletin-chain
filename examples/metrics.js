// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Performance metrics tracking for test examples
 */

export class PerformanceMetrics {
    constructor() {
        this.startTime = 0;
        this.endTime = 0;
        this.uploadDuration = 0;
        this.fileSize = 0;
        this.numChunks = 0;
        this.throughputMBps = 0;
        this.retrievalDuration = 0;
    }

    startUpload() {
        this.startTime = Date.now();
    }

    endUpload() {
        this.endTime = Date.now();
        this.uploadDuration = this.endTime - this.startTime;

        if (this.uploadDuration > 0 && this.fileSize > 0) {
            const durationSeconds = this.uploadDuration / 1000;
            const sizeMB = this.fileSize / (1024 * 1024);
            this.throughputMBps = sizeMB / durationSeconds;
        }
    }

    setFileSize(bytes) {
        this.fileSize = bytes;
    }

    setNumChunks(count) {
        this.numChunks = count;
    }

    setRetrievalDuration(ms) {
        this.retrievalDuration = ms;
    }

    print() {
        console.log('\n📊 Performance Metrics:');
        console.log('─'.repeat(60));
        console.log(`   File size:         ${(this.fileSize / 1024 / 1024).toFixed(2)} MB`);
        console.log(`   Number of chunks:  ${this.numChunks}`);
        console.log(`   Upload duration:   ${(this.uploadDuration / 1000).toFixed(2)} seconds`);
        console.log(`   Throughput:        ${this.throughputMBps.toFixed(2)} MB/s`);
        console.log(`   Retrieval time:    ${(this.retrievalDuration / 1000).toFixed(2)} seconds`);
        console.log('─'.repeat(60));
    }
}
