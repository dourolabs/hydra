// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import { render, cleanup, act } from "@testing-library/react";
import { useRef } from "react";
import { useOutsideClick } from "../hooks/useOutsideClick";

function Harness({
  enabled,
  onClose,
}: {
  enabled: boolean;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDivElement | null>(null);
  useOutsideClick(ref, onClose, enabled);
  return (
    <div>
      <div ref={ref} data-testid="inside">
        <button data-testid="inside-button">inside</button>
      </div>
      <button data-testid="outside-button">outside</button>
    </div>
  );
}

afterEach(() => cleanup());

describe("useOutsideClick", () => {
  it("fires onClose on document mousedown outside the ref", () => {
    const onClose = vi.fn();
    const { getByTestId } = render(<Harness enabled onClose={onClose} />);
    act(() => {
      getByTestId("outside-button").dispatchEvent(
        new MouseEvent("mousedown", { bubbles: true }),
      );
    });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("does not fire onClose on mousedown inside the ref", () => {
    const onClose = vi.fn();
    const { getByTestId } = render(<Harness enabled onClose={onClose} />);
    act(() => {
      getByTestId("inside-button").dispatchEvent(
        new MouseEvent("mousedown", { bubbles: true }),
      );
    });
    expect(onClose).not.toHaveBeenCalled();
  });

  it("fires onClose on Escape keydown", () => {
    const onClose = vi.fn();
    render(<Harness enabled onClose={onClose} />);
    act(() => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("does not fire onClose on non-Escape keydowns", () => {
    const onClose = vi.fn();
    render(<Harness enabled onClose={onClose} />);
    act(() => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    });
    expect(onClose).not.toHaveBeenCalled();
  });

  it("detaches listeners when disabled", () => {
    const onClose = vi.fn();
    const { rerender } = render(<Harness enabled={false} onClose={onClose} />);
    act(() => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(onClose).not.toHaveBeenCalled();

    rerender(<Harness enabled onClose={onClose} />);
    act(() => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(onClose).toHaveBeenCalledTimes(1);

    rerender(<Harness enabled={false} onClose={onClose} />);
    act(() => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
