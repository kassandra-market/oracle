/**
 * A local mock Anthropic Messages API server (Task T2).
 *
 * Serves `POST /v1/messages` and returns a VALID Anthropic Messages response
 * whose shape EXACTLY matches what the runner's REAL `AnthropicProvider`
 * consumes in `parse_messages_response` (`runner/src/anthropic.rs`):
 *
 *   - **success:** `{ id, type:"message", role:"assistant", model:<echoed>,
 *     content:[{ type:"text", text:"<structured-output JSON>" }],
 *     stop_reason:"end_turn", usage:{...} }`. The single `text` block's content
 *     is the JSON the runner parses for the option index (e.g.
 *     `{"option_index": N}`); `parse_messages_response` concatenates the
 *     `type:"text" content blocks verbatim → `parse_option_index`.
 *   - **refusal:** `{ ..., content:[], stop_reason:"refusal",
 *     stop_details:{ type:"refusal", category, explanation } }`. The runner
 *     checks `stop_reason == "refusal"` FIRST and bails with the category.
 *
 * The endpoint is `/v1/messages` because the runner treats `ANTHROPIC_BASE_URL`
 * as the API base and appends `/v1/messages` (see T1's `resolve_messages_url`).
 *
 * The response is configurable per scenario via {@link MockAnthropic.setMode}
 * (a test sets "this oracle resolves to option N", or "refuse"). On a success
 * mode the requested `model` is echoed back so the runner's resolved model id
 * matches. NOT gated/imported by the default suite.
 */
import {
  createServer,
  type IncomingMessage,
  type Server,
  type ServerResponse,
} from "node:http";
import { type AddressInfo } from "node:net";

/** What the next `/v1/messages` response should be. */
export type MockMode =
  | {
      kind: "success";
      /** The option index to answer with (goes into the `text` block JSON). */
      optionIndex: number;
      /** Override the echoed model id (default: echo the request's `model`). */
      model?: string;
    }
  | {
      kind: "refusal";
      /** `stop_details.category` (default `"policy"`). */
      category?: string;
      /** `stop_details.explanation` (default a canned string). */
      explanation?: string;
    };

/** The last request body the mock received (for assertions, if needed). */
export interface CapturedRequest {
  model?: string;
  max_tokens?: number;
  system?: string;
  messages?: unknown;
  output_config?: unknown;
}

export class MockAnthropic {
  private mode: MockMode = { kind: "success", optionIndex: 0 };
  /** Every request body seen, in order (for test assertions). */
  readonly requests: CapturedRequest[] = [];

  private constructor(
    private readonly server: Server,
    /** The base URL to hand the runner via `ANTHROPIC_BASE_URL`. */
    readonly baseUrl: string,
  ) {}

  /** Start the server on an ephemeral localhost port. */
  static async start(): Promise<MockAnthropic> {
    let mock: MockAnthropic;
    const server = createServer((req, res) => {
      mock.handle(req, res).catch((e) => {
        res.statusCode = 500;
        res.end(JSON.stringify({ error: String(e) }));
      });
    });
    await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
    const { port } = server.address() as AddressInfo;
    mock = new MockAnthropic(server, `http://127.0.0.1:${port}`);
    return mock;
  }

  /** Set what the next response(s) should be. */
  setMode(mode: MockMode): void {
    this.mode = mode;
  }

  /** Resolve the configured answer to option N. */
  setOption(optionIndex: number, model?: string): void {
    this.mode = { kind: "success", optionIndex, model };
  }

  /** Make the server refuse (so the runner's refusal arm can be tested). */
  setRefusal(category = "policy", explanation = "Mock declined this request."): void {
    this.mode = { kind: "refusal", category, explanation };
  }

  private async handle(req: IncomingMessage, res: ServerResponse): Promise<void> {
    if (req.method !== "POST" || !req.url?.endsWith("/v1/messages")) {
      res.statusCode = 404;
      res.end(JSON.stringify({ type: "error", error: { type: "not_found_error", message: req.url } }));
      return;
    }

    const body = await readBody(req);
    let parsed: CapturedRequest = {};
    try {
      parsed = JSON.parse(body) as CapturedRequest;
    } catch {
      // leave parsed empty; the runner always sends valid JSON.
    }
    this.requests.push(parsed);

    res.statusCode = 200;
    res.setHeader("content-type", "application/json");
    res.end(JSON.stringify(this.buildResponse(parsed)));
  }

  /** Build the Messages API response body for the current mode. */
  private buildResponse(req: CapturedRequest): unknown {
    const model = (this.mode.kind === "success" && this.mode.model) || req.model || "claude-opus-4-8";
    const base = {
      id: "msg_mock_0001",
      type: "message",
      role: "assistant",
      model,
      usage: { input_tokens: 10, output_tokens: 5 },
    };

    if (this.mode.kind === "refusal") {
      return {
        ...base,
        content: [],
        stop_reason: "refusal",
        stop_details: {
          type: "refusal",
          category: this.mode.category ?? "policy",
          explanation: this.mode.explanation ?? "Mock declined this request.",
        },
      };
    }

    // success: the single text block IS the structured-output JSON the runner
    // captures verbatim and parses for the option index.
    return {
      ...base,
      content: [{ type: "text", text: JSON.stringify({ option_index: this.mode.optionIndex }) }],
      stop_reason: "end_turn",
    };
  }

  /** Shut the server down cleanly. */
  async stop(): Promise<void> {
    await new Promise<void>((resolve, reject) =>
      this.server.close((err) => (err ? reject(err) : resolve())),
    );
  }
}

/** Read the full request body as a string. */
function readBody(req: IncomingMessage): Promise<string> {
  return new Promise((resolve, reject) => {
    let data = "";
    req.setEncoding("utf8");
    req.on("data", (c) => (data += c));
    req.on("end", () => resolve(data));
    req.on("error", reject);
  });
}
