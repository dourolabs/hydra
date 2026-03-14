import { useState, useCallback } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button, Input } from "@metis/ui";
import { apiClient } from "../../api/client";
import { useToast } from "../toast/useToast";
import styles from "./SecretsSection.module.css";

const SECRET_NAME_PATTERN = /^[A-Z][A-Z0-9_]{0,127}$/;

function validateSecretName(name: string): string | null {
  if (!SECRET_NAME_PATTERN.test(name)) {
    return "Must be 1-128 chars, start with uppercase letter, only uppercase letters/digits/underscores";
  }
  if (name.startsWith("METIS_")) {
    return "Names starting with METIS_ are reserved";
  }
  return null;
}

interface AddSecretFormProps {
  username: string;
  existingNames: string[];
}

export function AddSecretForm({ username, existingNames }: AddSecretFormProps) {
  const { addToast } = useToast();
  const queryClient = useQueryClient();
  const [adding, setAdding] = useState(false);
  const [name, setName] = useState("");
  const [value, setValue] = useState("");
  const [nameError, setNameError] = useState<string | null>(null);

  const setMutation = useMutation({
    mutationFn: ({ secretName, secretValue }: { secretName: string; secretValue: string }) =>
      apiClient.setSecret(username, secretName, secretValue),
    onSuccess: (_data, variables) => {
      queryClient.invalidateQueries({ queryKey: ["secrets"] });
      addToast(`${variables.secretName} saved`, "success");
      setAdding(false);
      setName("");
      setValue("");
      setNameError(null);
    },
    onError: (err) => {
      addToast(
        err instanceof Error ? err.message : "Failed to save secret",
        "error",
      );
    },
  });

  const handleNameChange = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    const newName = e.target.value.toUpperCase().replace(/[^A-Z0-9_]/g, "");
    setName(newName);
    if (newName.length > 0) {
      setNameError(validateSecretName(newName));
    } else {
      setNameError(null);
    }
  }, []);

  const handleSave = useCallback(() => {
    const trimmedName = name.trim();
    const trimmedValue = value.trim();
    if (trimmedName.length === 0 || trimmedValue.length === 0) return;

    const error = validateSecretName(trimmedName);
    if (error) {
      setNameError(error);
      return;
    }

    if (existingNames.includes(trimmedName)) {
      setNameError("A secret with this name already exists. Use the Update button instead.");
      return;
    }

    setMutation.mutate({ secretName: trimmedName, secretValue: trimmedValue });
  }, [name, value, existingNames, setMutation]);

  const handleCancel = useCallback(() => {
    setAdding(false);
    setName("");
    setValue("");
    setNameError(null);
  }, []);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        handleSave();
      } else if (e.key === "Escape") {
        handleCancel();
      }
    },
    [handleSave, handleCancel],
  );

  if (!adding) {
    return (
      <div className={styles.addSecretRow}>
        <Button variant="ghost" size="sm" onClick={() => setAdding(true)}>
          + Add Secret
        </Button>
      </div>
    );
  }

  return (
    <div className={styles.addSecretForm} onKeyDown={handleKeyDown}>
      <Input
        placeholder="SECRET_NAME"
        value={name}
        onChange={handleNameChange}
        autoFocus
      />
      {nameError && <span className={styles.validationError}>{nameError}</span>}
      <Input
        type="password"
        placeholder="Secret value"
        value={value}
        onChange={(e) => setValue(e.target.value)}
      />
      <div className={styles.secretFormActions}>
        <Button
          variant="secondary"
          size="sm"
          onClick={handleCancel}
          disabled={setMutation.isPending}
        >
          Cancel
        </Button>
        <Button
          variant="primary"
          size="sm"
          onClick={handleSave}
          disabled={
            name.trim().length === 0 ||
            value.trim().length === 0 ||
            nameError !== null ||
            setMutation.isPending
          }
        >
          {setMutation.isPending ? "Saving..." : "Save"}
        </Button>
      </div>
    </div>
  );
}
