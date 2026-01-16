import './style.css';
import { Logger } from './logger';
import { IPFSClient } from './ipfs';

// Initialize logger
const logger = new Logger('logs');

// Global IPFS client instance
let ipfsClient: IPFSClient | null = null;

// DOM elements
const peerMultiaddrsTextarea = document.getElementById('peer-multiaddrs') as HTMLTextAreaElement;
const cidInput = document.getElementById('cid-input') as HTMLInputElement;
const fetchBtn = document.getElementById('fetch-btn') as HTMLButtonElement;
const clearLogsBtn = document.getElementById('clear-logs-btn') as HTMLButtonElement;
const statusDiv = document.getElementById('status') as HTMLDivElement;
const contentDiv = document.getElementById('content') as HTMLDivElement;

// Status indicator functions
function setStatus(message: string, type: 'info' | 'success' | 'error' | 'loading') {
  statusDiv.textContent = message;
  statusDiv.className = `status status-${type}`;
}

// Clear logs button
clearLogsBtn.addEventListener('click', () => {
  logger.clear();
  logger.info('Logs cleared');
});

// Fetch button handler
fetchBtn.addEventListener('click', async () => {
  const cid = cidInput.value.trim();

  // Validation
  if (!cid) {
    setStatus('Please enter a CID', 'error');
    logger.error('CID is required');
    return;
  }

  // Parse peer multiaddrs
  const multiaddrsText = peerMultiaddrsTextarea.value.trim();
  if (!multiaddrsText) {
    setStatus('Please enter at least one peer multiaddr', 'error');
    logger.error('Peer multiaddrs are required');
    return;
  }

  const peerMultiaddrs = multiaddrsText
    .split('\n')
    .map(line => line.trim())
    .filter(line => line.length > 0);

  if (peerMultiaddrs.length === 0) {
    setStatus('Please enter at least one peer multiaddr', 'error');
    logger.error('Peer multiaddrs are required');
    return;
  }

  logger.info(`Parsed ${peerMultiaddrs.length} peer multiaddr(s)`);

  // Disable button during fetch
  fetchBtn.disabled = true;
  setStatus('Fetching...', 'loading');
  contentDiv.innerHTML = '<p class="placeholder loading">Loading...</p>';

  try {
    // Stop existing client if any
    if (ipfsClient && ipfsClient.isInitialized()) {
      logger.info('Stopping existing IPFS client...');
      await ipfsClient.stop();
    }

    // Create new client
    ipfsClient = new IPFSClient({
      logger,
      peerMultiaddrs,
    });

    // Initialize
    logger.info('=== Starting new fetch operation ===');
    await ipfsClient.initialize();

    // Fetch data
    const result = await ipfsClient.fetchData(cid);

    // Display content
    displayContent(result);
    setStatus(
      result.isJSON ? 'Successfully fetched data (JSON)' : 'Successfully fetched data (raw bytes)',
      'success'
    );
  } catch (error) {
    const errorMessage = error instanceof Error ? error.message : String(error);
    setStatus(`Error: ${errorMessage}`, 'error');
    contentDiv.innerHTML = `
      <div class="error-display">
        <h3>Error</h3>
        <p>${errorMessage}</p>
      </div>
    `;
  } finally {
    fetchBtn.disabled = false;
  }
});

// Display content (JSON or raw hex)
function displayContent(result: { data: any; isJSON: boolean; rawHex?: string }) {
  contentDiv.innerHTML = '';

  const header = document.createElement('div');
  header.className = 'json-header';

  if (result.isJSON) {
    // Display formatted JSON
    const pre = document.createElement('pre');
    pre.className = 'json-display';

    try {
      pre.textContent = JSON.stringify(result.data, null, 2);
    } catch (error) {
      pre.textContent = 'Error: Could not stringify data';
      logger.error('Failed to stringify data', error);
    }

    // Add copy button
    const copyBtn = document.createElement('button');
    copyBtn.textContent = 'Copy JSON';
    copyBtn.className = 'copy-btn';
    copyBtn.addEventListener('click', () => {
      navigator.clipboard
        .writeText(pre.textContent || '')
        .then(() => {
          logger.info('JSON copied to clipboard');
          copyBtn.textContent = 'Copied!';
          setTimeout(() => {
            copyBtn.textContent = 'Copy JSON';
          }, 2000);
        })
        .catch(err => {
          logger.error('Failed to copy to clipboard', err);
        });
    });

    header.appendChild(copyBtn);
    contentDiv.appendChild(header);
    contentDiv.appendChild(pre);
    logger.info('JSON content displayed successfully');
  } else {
    // Display raw hex
    const hexDisplay = document.createElement('div');
    hexDisplay.className = 'hex-display-container';

    // Info section
    const info = document.createElement('div');
    info.className = 'hex-info';
    info.innerHTML = `
      <strong>Raw Bytes (Hex String)</strong><br>
      Size: ${result.rawHex ? result.rawHex.length / 2 : 0} bytes
    `;

    // Hex content
    const pre = document.createElement('pre');
    pre.className = 'hex-display';
    pre.textContent = result.rawHex || '';

    // Add copy button for hex
    const copyBtn = document.createElement('button');
    copyBtn.textContent = 'Copy Hex';
    copyBtn.className = 'copy-btn';
    copyBtn.addEventListener('click', () => {
      navigator.clipboard
        .writeText(result.rawHex || '')
        .then(() => {
          logger.info('Hex string copied to clipboard');
          copyBtn.textContent = 'Copied!';
          setTimeout(() => {
            copyBtn.textContent = 'Copy Hex';
          }, 2000);
        })
        .catch(err => {
          logger.error('Failed to copy to clipboard', err);
        });
    });

    header.appendChild(copyBtn);
    hexDisplay.appendChild(info);

    contentDiv.appendChild(header);
    contentDiv.appendChild(hexDisplay);
    contentDiv.appendChild(pre);
    logger.info('Raw hex content displayed successfully');
  }
}

// Allow Enter key to submit
cidInput.addEventListener('keypress', e => {
  if (e.key === 'Enter') {
    fetchBtn.click();
  }
});

// Initial log
logger.info('Application initialized - P2P Mode');
logger.info('Ready to fetch data from IPFS via P2P');
logger.debug('Security: Only localhost (127.0.0.1, ::1) connections are allowed');
