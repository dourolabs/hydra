import { useCallback } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { UseMutationResult } from "@tanstack/react-query";
import { useToast } from "../features/toast/useToast";

interface UseFormModalOptions<TInput, TOutput> {
  mutationFn: (input: TInput) => Promise<TOutput>;
  invalidateKeys: string[][];
  successMessage: string;
  onSuccess?: (data: TOutput) => void;
  onError?: (error: Error, variables: TInput, context: unknown) => void;
  onMutate?: (variables: TInput) => Promise<unknown> | unknown;
}

interface UseFormModalResult<TInput, TOutput> {
  mutation: UseMutationResult<TOutput, Error, TInput, unknown>;
  handleClose: (resetForm: () => void, onClose: () => void) => void;
  handleKeyDown: (e: React.KeyboardEvent, submitFn: () => void) => void;
  isPending: boolean;
}

export function useFormModal<TInput, TOutput>({
  mutationFn,
  invalidateKeys,
  successMessage,
  onSuccess: onSuccessCb,
  onError: onErrorCb,
  onMutate: onMutateCb,
}: UseFormModalOptions<TInput, TOutput>): UseFormModalResult<TInput, TOutput> {
  const { addToast } = useToast();
  const queryClient = useQueryClient();

  const mutation = useMutation<TOutput, Error, TInput, unknown>({
    mutationFn,
    onMutate: onMutateCb,
    onSuccess: (data) => {
      if (!onMutateCb) {
        for (const key of invalidateKeys) {
          queryClient.invalidateQueries({ queryKey: key });
        }
      }
      addToast(successMessage, "success");
      onSuccessCb?.(data);
    },
    onError: (err, variables, context) => {
      onErrorCb?.(err, variables, context);
      addToast(
        err instanceof Error ? err.message : "An error occurred",
        "error",
      );
    },
    onSettled: onMutateCb
      ? () => {
          for (const key of invalidateKeys) {
            queryClient.invalidateQueries({ queryKey: key });
          }
        }
      : undefined,
  });

  const handleClose = useCallback(
    (resetForm: () => void, onClose: () => void) => {
      if (!mutation.isPending) {
        resetForm();
        onClose();
      }
    },
    [mutation.isPending],
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent, submitFn: () => void) => {
      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        submitFn();
      }
    },
    [],
  );

  return {
    mutation,
    handleClose,
    handleKeyDown,
    isPending: mutation.isPending,
  };
}
