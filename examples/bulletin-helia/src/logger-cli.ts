import { BaseLogger, type LogEntry, type LogLevel } from "./logger-base.js"

export class CLILogger extends BaseLogger {
  log(level: LogLevel, message: string, data?: unknown): void {
    const entry: LogEntry = {
      timestamp: new Date(),
      level,
      message,
      data,
    }

    const consoleMessage = `[${this.formatTimestamp(entry.timestamp)}] [${level.toUpperCase()}] ${message}`

    switch (level) {
      case "error":
        console.error(consoleMessage)
        if (data) console.error(data)
        break
      case "warning":
        console.warn(consoleMessage)
        if (data) console.warn(data)
        break
      case "debug":
      case "network":
        console.debug(consoleMessage)
        if (data) console.debug(data)
        break
      default:
        console.log(consoleMessage)
        if (data) console.log(data)
    }
  }
}
