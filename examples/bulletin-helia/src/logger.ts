import { BaseLogger, type LogLevel, type LogEntry } from './logger-base';

export type { LogLevel, LogEntry };

export class Logger extends BaseLogger {
  private container: HTMLElement;
  private autoScroll: boolean = true;

  constructor(containerId: string) {
    super();
    const container = document.getElementById(containerId);
    if (!container) {
      throw new Error(`Logger container with id '${containerId}' not found`);
    }
    this.container = container;
  }

  private createLogElement(entry: LogEntry): HTMLElement {
    const logDiv = document.createElement('div');
    logDiv.className = `log-entry log-${entry.level}`;

    const timestamp = document.createElement('span');
    timestamp.className = 'log-timestamp';
    timestamp.textContent = this.formatTimestamp(entry.timestamp);

    const level = document.createElement('span');
    level.className = 'log-level';
    level.textContent = `[${entry.level.toUpperCase()}]`;

    const message = document.createElement('span');
    message.className = 'log-message';
    message.textContent = entry.message;

    logDiv.appendChild(timestamp);
    logDiv.appendChild(level);
    logDiv.appendChild(message);

    if (entry.data) {
      const data = document.createElement('pre');
      data.className = 'log-data';
      data.textContent =
        typeof entry.data === 'string' ? entry.data : JSON.stringify(entry.data, null, 2);
      logDiv.appendChild(data);
    }

    return logDiv;
  }

  log(level: LogLevel, message: string, data?: any): void {
    const entry: LogEntry = {
      timestamp: new Date(),
      level,
      message,
      data,
    };

    this.logs.push(entry);
    const logElement = this.createLogElement(entry);
    this.container.appendChild(logElement);

    if (this.autoScroll) {
      this.container.scrollTop = this.container.scrollHeight;
    }

    // Also log to console for debugging
    const consoleMessage = `[${this.formatTimestamp(entry.timestamp)}] ${message}`;
    switch (level) {
      case 'error':
        console.error(consoleMessage, data);
        break;
      case 'warning':
        console.warn(consoleMessage, data);
        break;
      case 'debug':
      case 'network':
        console.debug(consoleMessage, data);
        break;
      default:
        console.log(consoleMessage, data);
    }
  }

  clear(): void {
    this.logs = [];
    this.container.innerHTML = '';
  }

  setAutoScroll(enabled: boolean): void {
    this.autoScroll = enabled;
  }
}
