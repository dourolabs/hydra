import { describe, it, expect, vi } from "vitest";
import { QueryClient } from "@tanstack/react-query";
import { upsertInList } from "./useSSE";

interface TestItem {
  id: string;
  version: number;
  name: string;
}

interface TestResponse {
  items: TestItem[];
}

const getItems = (r: TestResponse) => r.items;
const wrapItems = (items: TestItem[]): TestResponse => ({ items });
const getId = (item: TestItem) => item.id;

function makeQueryClient(initialData: TestResponse): QueryClient {
  const qc = new QueryClient();
  qc.setQueryData(["test"], initialData);
  return qc;
}

describe("upsertInList", () => {
  it("updates an existing entity when the incoming version is newer", () => {
    const qc = makeQueryClient({
      items: [{ id: "a", version: 1, name: "old" }],
    });

    upsertInList(qc, ["test"], getItems, wrapItems, getId, "a", {
      id: "a",
      version: 2,
      name: "new",
    });

    const data = qc.getQueryData<TestResponse>(["test"]);
    expect(data?.items).toEqual([{ id: "a", version: 2, name: "new" }]);
  });

  it("updates when the incoming version equals the cached version", () => {
    const qc = makeQueryClient({
      items: [{ id: "a", version: 3, name: "current" }],
    });

    upsertInList(qc, ["test"], getItems, wrapItems, getId, "a", {
      id: "a",
      version: 3,
      name: "same-version",
    });

    // Equal-version events must still update the cache
    const data = qc.getQueryData<TestResponse>(["test"]);
    expect(data?.items[0].name).toBe("same-version");
  });

  it("does not update when the incoming version is older", () => {
    const qc = makeQueryClient({
      items: [{ id: "a", version: 5, name: "current" }],
    });

    upsertInList(qc, ["test"], getItems, wrapItems, getId, "a", {
      id: "a",
      version: 3,
      name: "stale",
    });

    const data = qc.getQueryData<TestResponse>(["test"]);
    expect(data?.items[0].name).toBe("current");
  });

  it("appends a new entity when it does not exist in the list", () => {
    const qc = makeQueryClient({
      items: [{ id: "a", version: 1, name: "existing" }],
    });

    upsertInList(qc, ["test"], getItems, wrapItems, getId, "b", {
      id: "b",
      version: 1,
      name: "new-entity",
    });

    const data = qc.getQueryData<TestResponse>(["test"]);
    expect(data?.items).toHaveLength(2);
    expect(data?.items[1]).toEqual({ id: "b", version: 1, name: "new-entity" });
  });

  it("returns old data unchanged when cache entry is undefined", () => {
    const qc = new QueryClient();
    // No data set for ["test"] — setQueriesData updater receives undefined

    upsertInList(qc, ["test"], getItems, wrapItems, getId, "a", {
      id: "a",
      version: 1,
      name: "new",
    });

    const data = qc.getQueryData<TestResponse>(["test"]);
    expect(data).toBeUndefined();
  });
});
