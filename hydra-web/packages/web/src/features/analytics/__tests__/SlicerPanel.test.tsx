// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, cleanup, act, fireEvent } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

vi.mock("../../../api/client", () => ({
  apiClient: {
    listProjects: () => Promise.resolve({ projects: [] }),
    getProjectStatuses: () => Promise.resolve({ statuses: [] }),
    listRepositories: () => Promise.resolve({ repositories: [] }),
  },
}));

vi.mock("../SlicerPanel.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("@hydra/ui", () => ({
  Panel: ({ children, header }: { children: React.ReactNode; header?: React.ReactNode }) => (
    <div data-testid="panel">
      {header !== undefined && <div data-testid="panel-header">{header}</div>}
      {children}
    </div>
  ),
  Select: ({
    label,
    options,
    id,
    ...props
  }: {
    label?: string;
    id?: string;
    options: { value: string; label: string }[];
    [key: string]: unknown;
  }) => {
    const selectId = id ?? label?.toLowerCase().replace(/\s+/g, "-");
    return (
      <div>
        {label && <label htmlFor={selectId}>{label}</label>}
        <select id={selectId} {...(props as Record<string, unknown>)}>
          {options.map((opt) => (
            <option key={opt.value} value={opt.value}>
              {opt.label}
            </option>
          ))}
        </select>
      </div>
    );
  },
  Input: ({ label, id, ...props }: { label?: string; id?: string; [key: string]: unknown }) => {
    const inputId = id ?? label?.toLowerCase().replace(/\s+/g, "-");
    return (
      <div>
        {label && <label htmlFor={inputId}>{label}</label>}
        <input id={inputId} {...(props as Record<string, unknown>)} />
      </div>
    );
  },
}));

const { SlicerPanel } = await import("../SlicerPanel");
import type { SlicerState } from "../slicerState";

function makeQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
}

const baseState: SlicerState = {
  range: "30d",
  projectId: null,
  statusKeys: [],
  repoName: null,
  issueTypes: [],
  assignee: null,
  creator: null,
};

function renderPanel(state: SlicerState, onChange: (patch: Partial<SlicerState>) => void) {
  return render(
    <QueryClientProvider client={makeQueryClient()}>
      <SlicerPanel state={state} onChange={onChange} />
    </QueryClientProvider>,
  );
}

describe("SlicerPanel — debounced text inputs", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
    cleanup();
  });

  it("does not call onChange on each keystroke into Assignee", () => {
    const onChange = vi.fn();
    renderPanel(baseState, onChange);

    const input = screen.getByTestId("slicer-assignee") as HTMLInputElement;
    act(() => {
      fireEvent.change(input, { target: { value: "a" } });
      fireEvent.change(input, { target: { value: "al" } });
      fireEvent.change(input, { target: { value: "ali" } });
    });

    expect(onChange).not.toHaveBeenCalled();
    expect(input.value).toBe("ali");
  });

  it("commits the final Assignee value after the debounce window elapses", () => {
    const onChange = vi.fn();
    renderPanel(baseState, onChange);

    const input = screen.getByTestId("slicer-assignee") as HTMLInputElement;
    act(() => {
      fireEvent.change(input, { target: { value: "a" } });
      fireEvent.change(input, { target: { value: "al" } });
      fireEvent.change(input, { target: { value: "ali" } });
    });

    act(() => {
      vi.advanceTimersByTime(300);
    });

    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange).toHaveBeenCalledWith({ assignee: "ali" });
  });

  it("commits null when Assignee is cleared", () => {
    const onChange = vi.fn();
    renderPanel({ ...baseState, assignee: "alice" }, onChange);

    const input = screen.getByTestId("slicer-assignee") as HTMLInputElement;
    act(() => {
      fireEvent.change(input, { target: { value: "" } });
      vi.advanceTimersByTime(300);
    });

    expect(onChange).toHaveBeenCalledWith({ assignee: null });
  });

  it("debounces Creator the same way", () => {
    const onChange = vi.fn();
    renderPanel(baseState, onChange);

    const input = screen.getByTestId("slicer-creator") as HTMLInputElement;
    act(() => {
      fireEvent.change(input, { target: { value: "b" } });
      fireEvent.change(input, { target: { value: "bo" } });
      fireEvent.change(input, { target: { value: "bob" } });
    });
    expect(onChange).not.toHaveBeenCalled();

    act(() => {
      vi.advanceTimersByTime(300);
    });

    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange).toHaveBeenCalledWith({ creator: "bob" });
  });

  it("resyncs Assignee draft when state.assignee changes externally", () => {
    const onChange = vi.fn();
    const { rerender } = render(
      <QueryClientProvider client={makeQueryClient()}>
        <SlicerPanel state={baseState} onChange={onChange} />
      </QueryClientProvider>,
    );

    rerender(
      <QueryClientProvider client={makeQueryClient()}>
        <SlicerPanel state={{ ...baseState, assignee: "carol" }} onChange={onChange} />
      </QueryClientProvider>,
    );

    const input = screen.getByTestId("slicer-assignee") as HTMLInputElement;
    expect(input.value).toBe("carol");

    act(() => {
      vi.advanceTimersByTime(300);
    });
    expect(onChange).not.toHaveBeenCalled();
  });
});
