import { useEffect, useState } from "react";
import type { VersionResponse } from "@metis/api";
import { RepositoriesSection } from "../features/repositories/RepositoriesSection";
import { AgentsSection } from "../features/agents/AgentsSection";
import { SecretsSection } from "../features/secrets/SecretsSection";
import { apiClient } from "../api/client";
import styles from "./SettingsPage.module.css";

export function SettingsPage() {
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    apiClient
      .getVersion()
      .then((res: VersionResponse) => setVersion(res.version))
      .catch(() => {
        // silently ignore – footer will stay hidden
      });
  }, []);

  return (
    <div className={styles.page}>
      <RepositoriesSection />
      <AgentsSection />
      <SecretsSection />
      {version && (
        <footer className={styles.versionFooter}>{version}</footer>
      )}
    </div>
  );
}
