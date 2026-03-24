import { useCallback } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { UseMutationResult } from "@tanstack/react-query";
import { useToast } from "../features/toast/useToast";

interface UseFormModalOptions<TInput, TOutput, TContext> {
  mutationFn: (input: TInput) => Promise<TOutput>;
  invalidateKeys: string[][];
  successMessage: string | ((data: TOutput) => string);
  onSuccess?: (data: TOutput) => void;
  onError?: (error: Error, variables: TInput, context: TContext | undefined) => void;
  onMutate?: (variables: TInput) => Promise<TContext> | TContext;
}

interface UseFormModalResult<TInput, TOutput, TContext> {
  mutation: UseMutationResult<TOutput, Error, TInput, TContext>;
  handleClose: (onClose: () => void, resetForm?: () => void) => void;
  handleKeyDown: (e: React.KeyboardEvent, submitFn: () => void) => void;
  isPending: boolean;
}

export function useFormModal<TInput, TOutput, TContext = unknown>({
  mutationFn,
  invalidateKeys,
  successMessage,
  onSuccess: onSuccessCb,
  onError: onErrorCb,
  onMutate: onMutateCb,
}: UseFormModalOptions<TInput, TOutput, TContext>): UseFormModalResult<TInput, TOutput, TContext> {
  const { addToast } = useToast();
  const queryClient = useQueryClient();

  const mutation = useMutation<TOutput, Error, TInput, TContext>({
    mutationFn,
    onMutate: onMutateCb,
    onSuccess: (data) => {
      if (!onMutateCb) {
        for (const key of invalidateKeys) {
          queryClient.invalidateQueries({ queryKey: key });
        }
      }
      const message = typeof successMessage === "function" ? successMessage(data) : successMessage;
      addToast(message, "success");
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
    (onClose: () => void, resetForm?: () => void) => {
      if (!mutation.isPending) {
        resetForm?.();
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
