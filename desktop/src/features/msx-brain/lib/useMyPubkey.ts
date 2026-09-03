import { useIdentityQuery } from "@/shared/api/hooks";

/** Pubkey hiện tại của người dùng trong app — dùng cho phân quyền Não MSX. */
export function useMyPubkey(): string {
  const identityQuery = useIdentityQuery();
  return identityQuery.data?.pubkey ?? "";
}
