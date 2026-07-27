// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

/**
 * Unified logging functions for example scripts
 * Provides consistent formatting and visual hierarchy across all tests
 */

/**
 * Print a section header with double-line border
 * @param {string} text - Header text
 */
export function logHeader(text) {
    console.log('\n' + '═'.repeat(80));
    console.log(`  ${text}`);
    console.log('═'.repeat(80));
}

/**
 * Print a sub-section header with single-line border
 * @param {string} text - Section text
 */
export function logSection(text) {
    console.log('\n' + '─'.repeat(80));
    console.log(`  ${text}`);
    console.log('─'.repeat(80));
}

/**
 * Log configuration parameters in a formatted table
 * @param {Object} config - Configuration object with key-value pairs
 */
export function logConfig(config) {
    console.log('\n📋 Configuration:');
    for (const [key, value] of Object.entries(config)) {
        console.log(`   ${key.padEnd(20)}: ${value}`);
    }
}

/**
 * Log connection information (convenience function)
 * @param {string} wsUrl - WebSocket URL
 * @param {string} seed - Account seed or address
 * @param {string} ipfsApi - IPFS API URL
 */
export function logConnection(wsUrl, seed, ipfsApi) {
    logConfig({
        'RPC Endpoint': wsUrl,
        'Account/Seed': seed,
        'IPFS API': ipfsApi
    });
}

/**
 * Log a step in the process
 * @param {string} step - Step indicator (e.g., "1️⃣", "2️⃣")
 * @param {string} message - Step description
 */
export function logStep(step, message) {
    console.log(`\n${step} ${message}`);
}

/**
 * Log success message
 * @param {string} message - Success message
 */
export function logSuccess(message) {
    console.log(`✅ ${message}`);
}

/**
 * Log error message
 * @param {string} message - Error message
 */
export function logError(message) {
    console.error(`❌ ${message}`);
}

/**
 * Log info message
 * @param {string} message - Info message
 */
export function logInfo(message) {
    console.log(`ℹ️  ${message}`);
}

/**
 * Log warning message
 * @param {string} message - Warning message
 */
export function logWarning(message) {
    console.log(`⚠️  ${message}`);
}

/**
 * Log final test result with banner
 * @param {boolean} passed - Whether the test passed
 * @param {string} testName - Name of the test (default: "Test")
 */
export function logTestResult(passed, testName = 'Test') {
    console.log('\n' + '═'.repeat(80));
    if (passed) {
        console.log(`  ✅✅✅ ${testName} PASSED! ✅✅✅`);
    } else {
        console.log(`  ❌❌❌ ${testName} FAILED! ❌❌❌`);
    }
    console.log('═'.repeat(80) + '\n');
}
