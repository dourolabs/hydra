import { useCallback } from "react";
import type React from "react";

export interface KeyboardClickProps {
  onKeyDown: (e: React.KeyboardEvent) => void;
  role: "button";
  tabIndex: 0;
}

export function useKeyboardClick(handler: () => void): KeyboardClickProps {
  const onKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        handler();
      }
    },
    [handler],
  );

  return { onKeyDown, role: "button", tabIndex: 0 };
}
