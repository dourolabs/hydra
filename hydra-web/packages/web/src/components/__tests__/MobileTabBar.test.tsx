import { describe, it, expect, vi, afterEach } from "vitest";
import { render, cleanup, fireEvent, screen } from "@testing-library/react";

vi.mock("../MobileTabBar.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const { MobileTabBar } = await import("../MobileTabBar");

const TABS = [
  { key: "a", label: "Alpha" },
  { key: "b", label: "Bravo" },
  { key: "c", label: "Charlie" },
];

describe("MobileTabBar", () => {
  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
  });

  it("renders one button per tab with the expected labels", () => {
    render(<MobileTabBar tabs={TABS} activeKey="a" onChange={() => {}} />);
    const buttons = screen.getAllByRole("tab");
    expect(buttons).toHaveLength(3);
    expect(buttons[0].textContent).toBe("Alpha");
    expect(buttons[1].textContent).toBe("Bravo");
    expect(buttons[2].textContent).toBe("Charlie");
  });

  it("marks only the active tab via aria-selected", () => {
    render(<MobileTabBar tabs={TABS} activeKey="b" onChange={() => {}} />);
    const buttons = screen.getAllByRole("tab");
    expect(buttons[0].getAttribute("aria-selected")).toBe("false");
    expect(buttons[1].getAttribute("aria-selected")).toBe("true");
    expect(buttons[2].getAttribute("aria-selected")).toBe("false");
  });

  it("calls onChange with the clicked tab's key", () => {
    const onChange = vi.fn();
    render(<MobileTabBar tabs={TABS} activeKey="a" onChange={onChange} />);
    fireEvent.click(screen.getByText("Charlie"));
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange).toHaveBeenCalledWith("c");
  });

  it("uses the supplied testIdPrefix on each button", () => {
    render(
      <MobileTabBar tabs={TABS} activeKey="a" onChange={() => {}} testIdPrefix="my-prefix-" />,
    );
    expect(screen.getByTestId("my-prefix-a")).toBeTruthy();
    expect(screen.getByTestId("my-prefix-b")).toBeTruthy();
    expect(screen.getByTestId("my-prefix-c")).toBeTruthy();
  });

  it("renders the trailingAction node when provided", () => {
    render(
      <MobileTabBar
        tabs={TABS}
        activeKey="a"
        onChange={() => {}}
        trailingAction={<button data-testid="trailing">Action</button>}
      />,
    );
    expect(screen.getByTestId("trailing")).toBeTruthy();
  });

  it("omits the trailingAction wrapper when no node is provided", () => {
    const { container } = render(
      <MobileTabBar tabs={TABS} activeKey="a" onChange={() => {}} />,
    );
    // Only the three tab buttons should be children of the bar.
    expect(container.querySelectorAll('[role="tab"]')).toHaveLength(3);
    expect(container.querySelector(".trailing")).toBeNull();
  });
});
