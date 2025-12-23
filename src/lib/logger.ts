/**
 * Frontend logging utility using Tauri's logging plugin
 *
 * Provides consistent logging interface across the frontend application.
 * All logs are sent to the Tauri backend and handled by the logging system.
 */

import { trace, debug, info, warn, error } from "@tauri-apps/plugin-log";

/**
 * Log levels matching backend logging levels
 */
export enum LogLevel {
  TRACE = "trace",
  DEBUG = "debug",
  INFO = "info",
  WARN = "warn",
  ERROR = "error",
}

/**
 * Logger class for consistent frontend logging
 */
class Logger {
  /**
   * Log a trace-level message (most verbose)
   * Use for detailed execution flow
   */
  async trace(message: string, context?: Record<string, any>): Promise<void> {
    const logMessage = this.formatMessage(message, context);
    await trace(logMessage);
    if (import.meta.env.DEV) {
      console.trace("[TRACE]", message, context);
    }
  }

  /**
   * Log a debug-level message
   * Use for debugging information
   */
  async debug(message: string, context?: Record<string, any>): Promise<void> {
    const logMessage = this.formatMessage(message, context);
    await debug(logMessage);
    if (import.meta.env.DEV) {
      console.debug("[DEBUG]", message, context);
    }
  }

  /**
   * Log an info-level message
   * Use for important user actions and state changes
   */
  async info(message: string, context?: Record<string, any>): Promise<void> {
    const logMessage = this.formatMessage(message, context);
    await info(logMessage);
    if (import.meta.env.DEV) {
      console.info("[INFO]", message, context);
    }
  }

  /**
   * Log a warning message
   * Use for recoverable errors or unexpected situations
   */
  async warn(message: string, context?: Record<string, any>): Promise<void> {
    const logMessage = this.formatMessage(message, context);
    await warn(logMessage);
    console.warn("[WARN]", message, context);
  }

  /**
   * Log an error message
   * Use for errors and exceptions
   */
  async error(
    message: string,
    error?: Error | unknown,
    context?: Record<string, any>
  ): Promise<void> {
    let errorDetails = "";

    if (error instanceof Error) {
      errorDetails = ` | Error: ${error.message}`;
      if (error.stack && import.meta.env.DEV) {
        errorDetails += ` | Stack: ${error.stack}`;
      }
    } else if (error) {
      errorDetails = ` | Error: ${JSON.stringify(error)}`;
    }

    const logMessage = this.formatMessage(message + errorDetails, context);
    await error(logMessage);
    console.error("[ERROR]", message, error, context);
  }

  /**
   * Log user action (info level with user context)
   */
  async logUserAction(
    action: string,
    details?: Record<string, any>
  ): Promise<void> {
    await this.info(`User Action: ${action}`, details);
  }

  /**
   * Log component lifecycle event
   */
  async logComponentEvent(
    componentName: string,
    event: string,
    details?: Record<string, any>
  ): Promise<void> {
    await this.debug(`Component ${componentName}: ${event}`, details);
  }

  /**
   * Log API call
   */
  async logApiCall(
    method: string,
    endpoint: string,
    details?: Record<string, any>
  ): Promise<void> {
    await this.debug(`API Call: ${method} ${endpoint}`, details);
  }

  /**
   * Log API response
   */
  async logApiResponse(
    endpoint: string,
    success: boolean,
    details?: Record<string, any>
  ): Promise<void> {
    if (success) {
      await this.debug(`API Response: ${endpoint} - Success`, details);
    } else {
      await this.warn(`API Response: ${endpoint} - Failed`, details);
    }
  }

  /**
   * Log state change
   */
  async logStateChange(
    storeName: string,
    action: string,
    details?: Record<string, any>
  ): Promise<void> {
    await this.debug(`State Change [${storeName}]: ${action}`, details);
  }

  /**
   * Format message with context
   */
  private formatMessage(
    message: string,
    context?: Record<string, any>
  ): string {
    if (!context || Object.keys(context).length === 0) {
      return message;
    }

    try {
      const contextStr = Object.entries(context)
        .map(
          ([key, value]) =>
            `${key}=${
              typeof value === "object" ? JSON.stringify(value) : value
            }`
        )
        .join(", ");
      return `${message} | ${contextStr}`;
    } catch (e) {
      return message;
    }
  }
}

// Export singleton instance
export const logger = new Logger();

// Export convenience functions
export const logTrace = (message: string, context?: Record<string, any>) =>
  logger.trace(message, context);
export const logDebug = (message: string, context?: Record<string, any>) =>
  logger.debug(message, context);
export const logInfo = (message: string, context?: Record<string, any>) =>
  logger.info(message, context);
export const logWarn = (message: string, context?: Record<string, any>) =>
  logger.warn(message, context);
export const logError = (
  message: string,
  error?: Error | unknown,
  context?: Record<string, any>
) => logger.error(message, error, context);
export const logUserAction = (action: string, details?: Record<string, any>) =>
  logger.logUserAction(action, details);
export const logComponentEvent = (
  componentName: string,
  event: string,
  details?: Record<string, any>
) => logger.logComponentEvent(componentName, event, details);
export const logApiCall = (
  method: string,
  endpoint: string,
  details?: Record<string, any>
) => logger.logApiCall(method, endpoint, details);
export const logApiResponse = (
  endpoint: string,
  success: boolean,
  details?: Record<string, any>
) => logger.logApiResponse(endpoint, success, details);
export const logStateChange = (
  storeName: string,
  action: string,
  details?: Record<string, any>
) => logger.logStateChange(storeName, action, details);
