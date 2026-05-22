import type { TokenUsage } from "@hydra/api";
import { formatTokenCount } from "../../utils/tokens";
import styles from "./TokensCell.module.css";

interface TokensCellProps {
  usage?: TokenUsage | null;
}

export function TokensCell({ usage }: TokensCellProps) {
  if (!usage) {
    return <span className={styles.dash}>—</span>;
  }
  return (
    <span
      className={styles.tokens}
      title={`${usage.input_tokens} input · ${usage.cache_read_input_tokens} cache read · ${usage.cache_creation_input_tokens} cache creation · ${usage.output_tokens} output`}
    >
      <span className={styles.tokensInput}>
        <span aria-hidden="true">↓</span>
        {formatTokenCount(
          usage.input_tokens +
            usage.cache_read_input_tokens +
            usage.cache_creation_input_tokens,
        )}
      </span>
      <span className={styles.tokensOutput}>
        <span aria-hidden="true">↑</span>
        {formatTokenCount(usage.output_tokens)}
      </span>
    </span>
  );
}
