export type LogLevel = 'info' | 'success' | 'warning' | 'error' | 'debug' | 'network';

export interface LogEntry {
  timestamp: Date;
  level: LogLevel;
  message: string;
  data?: any;
}

export abstract class BaseLogger {
  protected logs: LogEntry[] = [];

  protected formatTimestamp(date: Date): string {
    return date.toLocaleTimeString('en-US', {
      hour12: false,
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
      fractionalSecondDigits: 3,
    });
  }

  abstract log(level: LogLevel, message: string, data?: any): void;
  abstract clear(): void;

  info(message: string, data?: any): void {
    this.log('info', message, data);
  }

  success(message: string, data?: any): void {
    this.log('success', message, data);
  }

  warning(message: string, data?: any): void {
    this.log('warning', message, data);
  }

  error(message: string, data?: any): void {
    this.log('error', message, data);
  }

  debug(message: string, data?: any): void {
    this.log('debug', message, data);
  }

  network(message: string, data?: any): void {
    this.log('network', message, data);
  }

  getLogs(): LogEntry[] {
    return [...this.logs];
  }
}
