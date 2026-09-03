import { sendChannelMessage } from "@/shared/api/tauriMessages";
import { getChannels } from "@/shared/api/tauriChannels";

const MAX_CHARS = 3000;

export function clipForChannel(content: string): string {
  const body = content.startsWith("---")
    ? content.split("---").slice(2).join("---").trim()
    : content;
  return body.length > MAX_CHARS ? body.slice(0, MAX_CHARS) : body;
}

export async function listShareableChannels(): Promise<Array<{ id: string; name: string }>> {
  const { channels } = await getChannels(null);
  return (channels ?? []).map((c) => ({ id: c.id, name: c.name }));
}

export async function shareToChannel(
  channelId: string,
  fileName: string,
  content: string,
): Promise<void> {
  await sendChannelMessage(channelId, `📄 **${fileName}**\n\n${clipForChannel(content)}`);
}
