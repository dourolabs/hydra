// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import React from "react";
import { MemoryRouter, useLocation } from "react-router-dom";

vi.mock("@hydra/ui", () => ({
  Tooltip: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  Kbd: ({ children }: { children: React.ReactNode }) => <kbd>{children}</kbd>,
  Button: ({
    children,
    ...props
  }: React.ButtonHTMLAttributes<HTMLButtonElement> & {
    variant?: string;
    size?: string;
  }) => {
    const { variant: _variant, size: _size, ...rest } = props;
    return <button {...rest}>{children}</button>;
  },
  Icons: new Proxy(
    {},
    {
      get: (_t, prop) => () => <span data-testid={`icon-${String(prop)}`} />,
    },
  ),
}));

const activeSessionCountMock = vi.fn();
vi.mock("../features/sessions/useActiveSessionCount", () => ({
  useActiveSessionCount: (...args: unknown[]) => activeSessionCountMock(...args),
}));

vi.mock("../features/auth/useAuth", () => ({
  useAuth: () => ({
    user: { actor: { type: "user", username: "alice" } },
    logout: vi.fn(),
    loading: false,
  }),
}));

vi.mock("../api/auth", () => ({
  actorDisplayName: () => "Alice",
}));

const openIssueCreateModalMock = vi.fn();
vi.mock("../features/dashboard/useIssueCreateModal", () => ({
  useIssueCreateModal: () => ({
    isOpen: false,
    open: openIssueCreateModalMock,
    close: vi.fn(),
  }),
}));

const openChatCreateModalMock = vi.fn();
vi.mock("../features/chat/useChatCreateModal", () => ({
  useChatCreateModal: () => ({
    isOpen: false,
    open: openChatCreateModalMock,
    close: vi.fn(),
  }),
}));

vi.mock("./SiteHeader.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("./HeaderActionMenu.module.css", () => ({
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
          <BreadcrumbsSetter items={opts.breadcrumbs.items} current={opts.breadcrumbs.current} />
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

type ChangeListener = (e: MediaQueryListEvent) => void;
function mockMatchMedia(matches: boolean) {
  const listeners: ChangeListener[] = [];
  const mql = {
    matches,
    media: "",
    onchange: null,
    addEventListener: (event: string, handler: ChangeListener) => {
      if (event === "change") listeners.push(handler);
    },
    removeEventListener: (event: string, handler: ChangeListener) => {
      if (event === "change") {
        const idx = listeners.indexOf(handler);
        if (idx !== -1) listeners.splice(idx, 1);
      }
    },
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => true,
  };
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    writable: true,
    value: () => mql as unknown as MediaQueryList,
  });
}

beforeEach(() => {
  activeSessionCountMock.mockReturnValue({ data: 0 });
  mockMatchMedia(false);
});

afterEach(() => {
  cleanup();
  openIssueCreateModalMock.mockReset();
  openChatCreateModalMock.mockReset();
});

describe("SiteHeader", () => {
  it("renders breadcrumbs, search, sessions pill, and create trigger", () => {
    renderHeader({
      breadcrumbs: { items: [], current: "Issues" },
    });
    expect(screen.getByTestId("site-header")).toBeTruthy();
    expect(screen.getByTestId("site-header-breadcrumbs")).toBeTruthy();
    expect(screen.getByTestId("site-header-search")).toBeTruthy();
    expect(screen.getByTestId("site-header-sessions")).toBeTruthy();
    expect(screen.getByTestId("site-header-create")).toBeTruthy();
  });

  it("hamburger toggles sidebar state on click", () => {
    const onHide = vi.fn();
    const onShow = vi.fn();
    renderHeader({ hidden: false, onHide, onShow });
    fireEvent.click(screen.getByTestId("site-header-toggle-sidebar"));
    expect(onHide).toHaveBeenCalled();
    cleanup();

    renderHeader({ hidden: true, onHide, onShow });
    fireEvent.click(screen.getByTestId("site-header-toggle-sidebar"));
    expect(onShow).toHaveBeenCalled();
  });

  it("invokes onOpenSearch when search button is clicked", () => {
    const onOpenSearch = vi.fn();
    renderHeader({ onOpenSearch });
    fireEvent.click(screen.getByTestId("site-header-search"));
    expect(onOpenSearch).toHaveBeenCalledTimes(1);
  });

  it("create trigger opens a menu with New issue and New conversation", () => {
    renderHeader();
    const trigger = screen.getByTestId("site-header-create");
    expect(trigger.getAttribute("aria-haspopup")).toBe("menu");
    expect(trigger.getAttribute("aria-expanded")).toBe("false");

    fireEvent.click(trigger);

    expect(trigger.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByTestId("site-header-create-menu")).toBeTruthy();
    expect(screen.getByTestId("site-header-new-issue")).toBeTruthy();
    expect(screen.getByTestId("site-header-new-conversation")).toBeTruthy();
  });

  it("selecting New issue opens the create-issue modal and closes the menu", () => {
    renderHeader();
    fireEvent.click(screen.getByTestId("site-header-create"));
    fireEvent.click(screen.getByTestId("site-header-new-issue"));
    expect(openIssueCreateModalMock).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId("site-header-create-menu")).toBeNull();
  });

  it("selecting New conversation opens the chat-create modal and closes the menu", () => {
    renderHeader();
    fireEvent.click(screen.getByTestId("site-header-create"));
    fireEvent.click(screen.getByTestId("site-header-new-conversation"));
    expect(openChatCreateModalMock).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId("site-header-create-menu")).toBeNull();
  });

  it("closes the menu on Escape", () => {
    renderHeader();
    fireEvent.click(screen.getByTestId("site-header-create"));
    const menu = screen.getByTestId("site-header-create-menu");
    fireEvent.keyDown(menu, { key: "Escape" });
    expect(screen.queryByTestId("site-header-create-menu")).toBeNull();
  });

  it("closes the menu on outside click", () => {
    renderHeader();
    fireEvent.click(screen.getByTestId("site-header-create"));
    expect(screen.getByTestId("site-header-create-menu")).toBeTruthy();
    fireEvent.mouseDown(document.body);
    expect(screen.queryByTestId("site-header-create-menu")).toBeNull();
  });

  it("renders the sessions pill as a link to /sessions", () => {
    renderHeader();
    const link = screen.getByTestId("site-header-sessions") as HTMLAnchorElement;
    expect(link.tagName).toBe("A");
    expect(link.getAttribute("href")).toBe("/sessions");
  });

  it("renders 'no sessions' label and inactive dot when count is zero", () => {
    activeSessionCountMock.mockReturnValue({ data: 0 });
    renderHeader();
    expect(screen.getByTestId("site-header-sessions-label").textContent).toBe("no sessions");
    expect(
      screen.getByTestId("site-header-sessions-dot").getAttribute("data-active"),
    ).toBe("false");
  });

  it("renders '1 session' and active dot when count is one", () => {
    activeSessionCountMock.mockReturnValue({ data: 1 });
    renderHeader();
    expect(screen.getByTestId("site-header-sessions-label").textContent).toBe("1 session");
    expect(
      screen.getByTestId("site-header-sessions-dot").getAttribute("data-active"),
    ).toBe("true");
  });

  it("renders pluralised sessions label when count > 1", () => {
    activeSessionCountMock.mockReturnValue({ data: 9 });
    renderHeader();
    expect(screen.getByTestId("site-header-sessions-label").textContent).toBe("9 sessions");
  });

  it("passes the current-user creator filter into useActiveSessionCount", () => {
    activeSessionCountMock.mockReturnValue({ data: 0 });
    renderHeader();
    expect(activeSessionCountMock).toHaveBeenCalledWith("Alice");
  });
});
