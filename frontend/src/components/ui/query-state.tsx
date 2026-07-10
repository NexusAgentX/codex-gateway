import type { UseQueryResult } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { ErrorState, LoadingState } from "./state";

export function QueryState<T>({ query, children }: { query: UseQueryResult<T, Error>; children: (data: T) => ReactNode }) {
  if (query.isLoading) {
    return <LoadingState />;
  }
  if (query.isError) {
    return <ErrorState error={query.error} />;
  }
  if (!query.data) {
    return <LoadingState />;
  }
  return <>{children(query.data)}</>;
}
