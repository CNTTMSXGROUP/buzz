import { sendChannelMessage } from "@/shared/api/tauriMessages";
import { getChannels } from "@/shared/api/tauriChannels";

export function stripGhiNhanh(fileName: string): string {
  return fileName.replace(/^GHI NHANH — /, "").replace(/\.md$/, "");
}

let dieuHanhCache: string | null = null;

export async function findDieuHanhChannelId(): Promise<string | null> {
  if (dieuHanhCache) return dieuHanhCache;
  const { channels } = await getChannels(null);
  const dh = channels?.find((c) => c.name === "dieu-hanh") ?? null;
  if (dh) dieuHanhCache = dh.id;
  return dieuHanhCache;
}

/** Gửi lệnh !duyet vào #dieu-hanh — bridge cron relay sẽ xử lý tiếp (~60s). */
export async function sendApprove(fileName: string): Promise<void> {
  const channelId = await findDieuHanhChannelId();
  if (!channelId) throw new Error("Không tìm thấy kênh dieu-hanh");
  await sendChannelMessage(channelId, `!duyet ${stripGhiNhanh(fileName)}`);
}
