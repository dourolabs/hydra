import { Hono } from "hono";
import type { Store } from "../store.js";
import type {
  AgentRecord,
  AgentResponse,
  UpsertAgentRequest,
  ListAgentsResponse,
  DeleteAgentResponse,
} from "@metis/api";

const COLLECTION = "agents";

export function createAgentRoutes(store: Store): Hono {
  const app = new Hono();

  // GET /v1/agents
  app.get("/v1/agents", (c) => {
    const items = store.list<AgentRecord>(COLLECTION);
    const agents: AgentRecord[] = items.map(({ entry }) => entry.data);
    const resp: ListAgentsResponse = { agents };
    return c.json(resp);
  });

  // GET /v1/agents/:name
  app.get("/v1/agents/:name", (c) => {
    const name = c.req.param("name");
    const entry = store.get<AgentRecord>(COLLECTION, name);
    if (!entry) {
      return c.json({ error: `agent '${name}' not found` }, 404);
    }
    const resp: AgentResponse = { agent: entry.data };
    return c.json(resp);
  });

  // POST /v1/agents
  app.post("/v1/agents", async (c) => {
    const body = await c.req.json<UpsertAgentRequest>();
    const agent: AgentRecord = {
      name: body.name,
      prompt: body.prompt,
      prompt_path: body.prompt_path,
      max_tries: body.max_tries,
      max_simultaneous: body.max_simultaneous,
      is_assignment_agent: body.is_assignment_agent,
    };
    store.create<AgentRecord>(COLLECTION, body.name, agent, null);
    const resp: AgentResponse = { agent };
    return c.json(resp, 201);
  });

  // PUT /v1/agents/:name
  app.put("/v1/agents/:name", async (c) => {
    const name = c.req.param("name");
    const body = await c.req.json<UpsertAgentRequest>();
    const agent: AgentRecord = {
      name: body.name,
      prompt: body.prompt,
      prompt_path: body.prompt_path,
      max_tries: body.max_tries,
      max_simultaneous: body.max_simultaneous,
      is_assignment_agent: body.is_assignment_agent,
    };
    store.update<AgentRecord>(COLLECTION, name, agent, null);
    const resp: AgentResponse = { agent };
    return c.json(resp);
  });

  // DELETE /v1/agents/:name
  app.delete("/v1/agents/:name", (c) => {
    const name = c.req.param("name");
    const entry = store.delete<AgentRecord>(COLLECTION, name, null);
    const resp: DeleteAgentResponse = { agent: entry.data };
    return c.json(resp);
  });

  return app;
}
