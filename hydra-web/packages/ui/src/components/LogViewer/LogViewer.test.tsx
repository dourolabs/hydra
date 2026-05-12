import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import type { ComponentType } from "react";

vi.mock("./LogViewer.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

interface RowProps {
  index: number;
  style: object;
  getHtml: (index: number) => string;
  lineNumberWidth: string;
}

vi.mock("react-window", () => ({
  List: ({
    rowCount,
    rowComponent: RowComponent,
    rowProps,
  }: {
    rowCount: number;
    rowComponent: ComponentType<RowProps>;
    rowProps: { getHtml: (index: number) => string; lineNumberWidth: string };
    rowHeight?: number;
    overscanCount?: number;
    style?: object;
    listRef?: unknown;
  }) => (
    <div>
      {Array.from({ length: rowCount }, (_, i) => (
        <RowComponent key={i} index={i} style={{}} {...rowProps} />
      ))}
    </div>
  ),
  useListRef: () => ({ current: null }),
}));

const { LogViewer } = await import("./LogViewer");

describe("LogViewer", () => {
  it("renders one row per input line, even when the line contains embedded \\r", () => {
    render(<LogViewer lines={["Progress: 10%\rProgress: 50%\rProgress: 100%"]} />);

    const rows = screen.getAllByTestId("log-row");
    expect(rows).toHaveLength(1);

    // The carriage returns should have been stripped before rendering, so the
    // resulting text contains the concatenated progress markers without any
    // bare CR that the browser would render as a line break.
    expect(rows[0].textContent).toContain("Progress: 10%Progress: 50%Progress: 100%");
    expect(rows[0].innerHTML).not.toContain("\r");
  });

  it("renders one row per line for a multi-line input", () => {
    render(<LogViewer lines={["first", "second", "third"]} />);
    expect(screen.getAllByTestId("log-row")).toHaveLength(3);
  });

  it("strips CRs from CRLF-terminated lines", () => {
    render(<LogViewer lines={["line one\r"]} />);
    const rows = screen.getAllByTestId("log-row");
    expect(rows).toHaveLength(1);
    expect(rows[0].innerHTML).not.toContain("\r");
  });

  it("renders the empty placeholder when no lines are provided", () => {
    render(<LogViewer lines={[]} />);
    expect(screen.getByText("No log output")).toBeDefined();
    expect(screen.queryAllByTestId("log-row")).toHaveLength(0);
  });
});
