import { type ButtonHTMLAttributes, Children } from "react";
import styles from "./Button.module.css";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "primary" | "secondary" | "ghost" | "danger" | "danger-subtle";
  size?: "sm" | "md" | "lg";
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

  return (
    <button className={cls} {...props}>
      {children}
    </button>
  );
}
