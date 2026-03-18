import type { ApiErrorBody } from "./generated/ApiErrorBody";

/**
 * Error thrown when a hydra-server API request returns a non-2xx status.
 */
export class ApiError extends Error {
  constructor(
    public readonly status: number,
    message: string,
  ) {
    super(message);
    this.name = "ApiError";
  }

  /**
   * Build an ApiError from a failed fetch Response.
   */
  static async fromResponse(response: Response): Promise<ApiError> {
    let message: string;
    try {
      const body: ApiErrorBody = await response.json();
      message = body.error;
    } catch {
      message = response.statusText || `HTTP ${response.status}`;
    }
    return new ApiError(response.status, message);
  }
}
