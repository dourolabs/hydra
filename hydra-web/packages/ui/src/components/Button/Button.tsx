import {
  type ButtonHTMLAttributes,
  Children,
  cloneElement,
  isValidElement,
  type ReactElement,
} from "react";
import styles from "./Button.module.css";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "primary" | "secondary" | "ghost" | "danger" | "danger-subtle";
  size?: "sm" | "md" | "lg";
  /**
   * Render Button's styling on the single React child instead of a native
   * `<button>`. Used to project the canonical button styling onto an element
   * with different semantics — most commonly a react-router `<Link>` so a
   * navigation affordance shares the same hover/focus/touch-target rules as
   * a real button without duplicating the CSS.
   */
  asChild?: boolean;
}

function getTextLength(children: React.ReactNode): number {
  return Children.toArray(children).reduce<number>((acc, child) => {
    if (typeof child === "string" || typeof child === "number") {
      return acc + String(child).trim().length;
    }
    return acc;
  }, 0);
}

export function Button({
  variant = "primary",
  size = "md",
  asChild = false,
  className,
  children,
  ...props
}: ButtonProps) {
  const isIconOnly = getTextLength(children) === 0;

  const cls = [
    styles.button,
    styles[variant],
    styles[size],
    isIconOnly && styles.iconOnly,
    className,
  ]
    .filter(Boolean)
    .join(" ");

  if (asChild) {
    const child = Children.only(children);
    if (!isValidElement<{ className?: string }>(child)) {
      throw new Error("<Button asChild> expects a single React element child");
    }
    const merged = [child.props.className, cls].filter(Boolean).join(" ");
    return cloneElement(child as ReactElement<{ className?: string }>, {
      className: merged,
    });
  }

  return (
    <button className={cls} {...props}>
      {children}
    </button>
  );
}
