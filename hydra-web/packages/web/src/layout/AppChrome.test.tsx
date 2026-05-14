// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";

vi.mock("./AppLayout.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("./HydraBrand.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const { AppChrome } = await import("./AppChrome");

afterEach(() => {
  cleanup();
});

describe("AppChrome", () => {
  it("renders the hamburger and the Hydra wordmark", () => {
    render(<AppChrome hidden={false} onHide={() => {}} onShow={() => {}} />);
    expect(screen.getByTestId("app-chrome-toggle-sidebar")).toBeTruthy();
    expect(screen.getByTestId("hydra-brand")).toBeTruthy();
    expect(screen.getByTestId("hydra-brand").textContent).toContain("Hydra");
  });

  it("labels the hamburger 'Hide sidebar' when the sidebar is open", () => {
    render(<AppChrome hidden={false} onHide={() => {}} onShow={() => {}} />);
    expect(
      screen.getByTestId("app-chrome-toggle-sidebar").getAttribute("aria-label"),
    ).toBe("Hide sidebar");
  });

  it("labels the hamburger 'Show sidebar' when the sidebar is hidden", () => {
    render(<AppChrome hidden={true} onHide={() => {}} onShow={() => {}} />);
    expect(
      screen.getByTestId("app-chrome-toggle-sidebar").getAttribute("aria-label"),
    ).toBe("Show sidebar");
  });

  it("calls onHide when toggled while the sidebar is open", () => {
    const onHide = vi.fn();
    const onShow = vi.fn();
    render(<AppChrome hidden={false} onHide={onHide} onShow={onShow} />);
    fireEvent.click(screen.getByTestId("app-chrome-toggle-sidebar"));
    expect(onHide).toHaveBeenCalledTimes(1);
    expect(onShow).not.toHaveBeenCalled();
  });

  it("calls onShow when toggled while the sidebar is hidden", () => {
    const onHide = vi.fn();
    const onShow = vi.fn();
    render(<AppChrome hidden={true} onHide={onHide} onShow={onShow} />);
    fireEvent.click(screen.getByTestId("app-chrome-toggle-sidebar"));
    expect(onShow).toHaveBeenCalledTimes(1);
    expect(onHide).not.toHaveBeenCalled();
  });

  it("marks the chrome with leftChromeOnHeader when the sidebar is hidden", () => {
    // The chrome carries the SiteHeader's bottom border only when it visually
    // belongs to the header (sidebar collapsed).
    const { unmount } = render(
      <AppChrome hidden={false} onHide={() => {}} onShow={() => {}} />,
    );
    expect(screen.getByTestId("app-left-chrome").className).not.toContain(
      "leftChromeOnHeader",
    );
    unmount();
    render(<AppChrome hidden={true} onHide={() => {}} onShow={() => {}} />);
    expect(screen.getByTestId("app-left-chrome").className).toContain(
      "leftChromeOnHeader",
    );
  });
});
