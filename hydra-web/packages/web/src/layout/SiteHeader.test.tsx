// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup, act } from "@testing-library/react";
import React from "react";
import { MemoryRouter, useLocation } from "react-router-dom";

vi.mock("@hydra/ui", () => ({
  Tooltip: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

const activeSessionCountMock = vi.fn();
vi.mock("../features/sessions/useActiveSessionCount", () => ({
  useActiveSessionCount: () => activeSessionCountMock(),
}));

vi.mock("./SiteHeader.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("./Breadcrumbs.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

const { SiteHeader } = await import("./SiteHeader");
const { BreadcrumbsProvider } = await import("./BreadcrumbsProvider");
const { useBreadcrumbs } = await import("./useBreadcrumbs");

function LocationDisplay() {
  const location = useLocation();
  return <div data-testid="location-pathname">{location.pathname}</div>;
}

interface RenderOpts {
  hidden?: boolean;
  onHide?: () => void;
  onShow?: () => void;
  onOpenSearch?: () => void;
  initialEntry?: string;
  breadcrumbs?: { items: { label: string; to: string }[]; current: string };
}

function BreadcrumbsSetter({
  items,
  current,
}: {
  items: { label: string; to: string }[];
  current: string;
}) {
  useBreadcrumbs(items, current);
  return null;
}

function renderHeader(opts: RenderOpts = {}) {
  return render(
    <MemoryRouter initialEntries={[opts.initialEntry ?? "/"]}>
      <BreadcrumbsProvider>
        {opts.breadcrumbs && (
          <BreadcrumbsSetter
            items={opts.breadcrumbs.items}
            current={opts.breadcrumbs.current}
          />
        )}
        <SiteHeader
          hidden={opts.hidden ?? false}
          onHide={opts.onHide ?? (() => {})}
          onShow={opts.onShow ?? (() => {})}
          onOpenSearch={opts.onOpenSearch ?? (() => {})}
        />
        <LocationDisplay />
      </BreadcrumbsProvider>
    </MemoryRouter>,
  );
}

beforeEach(() => {
  activeSessionCountMock.mockReturnValue({ data: 0 });
});

afterEach(() => {
  cleanup();
  activeSessionCountMock.mockReset();
});

describe("SiteHeader sidebar toggle", () => {
  it("calls onHide when sidebar is shown and the toggle is clicked", () => {
    const onHide = vi.fn();
    const onShow = vi.fn();
    renderHeader({ hidden: false, onHide, onShow });
    fireEvent.click(screen.getByTestId("site-header-toggle-sidebar"));
    expect(onHide).toHaveBeenCalledTimes(1);
    expect(onShow).not.toHaveBeenCalled();
  });

  it("calls onShow when sidebar is hidden and the toggle is clicked", () => {
    const onHide = vi.fn();
    const onShow = vi.fn();
    renderHeader({ hidden: true, onHide, onShow });
    fireEvent.click(screen.getByTestId("site-header-toggle-sidebar"));
    expect(onShow).toHaveBeenCalledTimes(1);
    expect(onHide).not.toHaveBeenCalled();
  });

  it("uses the right aria-label depending on hidden state", () => {
    const { unmount } = renderHeader({ hidden: false });
    expect(
      screen.getByTestId("site-header-toggle-sidebar").getAttribute("aria-label"),
    ).toBe("Hide sidebar");
    unmount();
    renderHeader({ hidden: true });
    expect(
      screen.getByTestId("site-header-toggle-sidebar").getAttribute("aria-label"),
    ).toBe("Show sidebar");
  });
});

describe("SiteHeader search button", () => {
  it("invokes onOpenSearch when clicked", () => {
    const onOpenSearch = vi.fn();
    renderHeader({ onOpenSearch });
    fireEvent.click(screen.getByTestId("site-header-search"));
    expect(onOpenSearch).toHaveBeenCalledTimes(1);
  });
});

describe("SiteHeader active sessions pill", () => {
  it("renders the sessions pill as a link to /sessions", () => {
    renderHeader();
    const slot = screen.getByTestId("site-header-sessions");
    expect(slot.tagName).toBe("A");
    expect(slot.getAttribute("href")).toBe("/sessions");
  });

  it("renders 'no sessions' with an inactive dot when count is zero", () => {
    activeSessionCountMock.mockReturnValue({ data: 0 });
    renderHeader();
    expect(screen.getByTestId("site-header-sessions-label").textContent).toBe(
      "no sessions",
    );
    expect(
      screen.getByTestId("site-header-sessions-dot").getAttribute("data-active"),
    ).toBe("false");
  });

  it("renders '1 session' with a pulsing dot when count is one", () => {
    activeSessionCountMock.mockReturnValue({ data: 1 });
    renderHeader();
    expect(screen.getByTestId("site-header-sessions-label").textContent).toBe(
      "1 session",
    );
    expect(
      screen.getByTestId("site-header-sessions-dot").getAttribute("data-active"),
    ).toBe("true");
  });

  it("renders 'N sessions' with a pulsing dot when count is greater than one", () => {
    activeSessionCountMock.mockReturnValue({ data: 4 });
    renderHeader();
    expect(screen.getByTestId("site-header-sessions-label").textContent).toBe(
      "4 sessions",
    );
    expect(
      screen.getByTestId("site-header-sessions-dot").getAttribute("data-active"),
    ).toBe("true");
  });

  it("treats an undefined count as zero (loading state)", () => {
    activeSessionCountMock.mockReturnValue({ data: undefined });
    renderHeader();
    expect(screen.getByTestId("site-header-sessions-label").textContent).toBe(
      "no sessions",
    );
    expect(
      screen.getByTestId("site-header-sessions-dot").getAttribute("data-active"),
    ).toBe("false");
  });

  it("navigates to /sessions when the sessions pill is clicked", () => {
    activeSessionCountMock.mockReturnValue({ data: 2 });
    renderHeader({ initialEntry: "/" });
    expect(screen.getByTestId("location-pathname").textContent).toBe("/");
    fireEvent.click(screen.getByTestId("site-header-sessions"));
    expect(screen.getByTestId("location-pathname").textContent).toBe("/sessions");
  });
});

describe("SiteHeader breadcrumbs", () => {
  it("renders the breadcrumbs slot empty when no page has published breadcrumbs", () => {
    renderHeader();
    const slot = screen.getByTestId("site-header-breadcrumbs");
    expect(slot.textContent).toBe("");
  });

  it("renders breadcrumbs published via the useBreadcrumbs hook", async () => {
    renderHeader({
      breadcrumbs: {
        items: [{ label: "Dashboard", to: "/" }],
        current: "Issue i-x",
      },
    });
    await act(async () => {});
    const slot = screen.getByTestId("site-header-breadcrumbs");
    expect(slot.textContent).toContain("Dashboard");
    expect(slot.textContent).toContain("Issue i-x");
    // Parent link should point to the items[].to
    const parentLink = slot.querySelector("a");
    expect(parentLink?.getAttribute("href")).toBe("/");
  });

  it("renders a single-segment breadcrumb when items is empty", async () => {
    renderHeader({
      breadcrumbs: { items: [], current: "Documents" },
    });
    await act(async () => {});
    const slot = screen.getByTestId("site-header-breadcrumbs");
    expect(slot.textContent).toContain("Documents");
    expect(slot.querySelector("a")).toBeNull();
  });
});
