/** Đường dẫn vault não mặc định — override bằng localStorage "msx-brain-vault-root". */
export const MSX_VAULT_ROOT_DEFAULT =
  "/Users/qthang/Library/CloudStorage/GoogleDrive-aios.msxgroup@gmail.com/Drive của tôi/MSXGROUP_AIOS_BRAIN";

export function getVaultRoot(): string {
  try {
    return localStorage.getItem("msx-brain-vault-root") ?? MSX_VAULT_ROOT_DEFAULT;
  } catch {
    return MSX_VAULT_ROOT_DEFAULT;
  }
}
