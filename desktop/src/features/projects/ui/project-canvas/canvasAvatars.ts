import { fetchAvatarBlob } from "@/features/profile/lib/selfProfileStorage";
import { getAvatarSnapshotUrl } from "@/shared/lib/animatedAvatar";
import {
  DEFAULT_AVATAR_PIXEL_SIZE,
  downscaleAvatarDataUrl,
} from "@/shared/lib/avatarDownscale";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";

/**
 * Avatar delivery policy for sandboxed Project Canvas frames.
 *
 * A canvas frame cannot reach the network, so the host fetches avatars itself
 * and hands them to widgets as `data:` URLs inside the RPC payload. Two
 * ceilings apply and they are not independent: each image is re-encoded small
 * enough to be worth sending, and the *sum* across a response has to stay
 * inside `PROJECT_CANVAS_MAX_PORT_MESSAGE_BYTES` (64 KiB), or the host replies
 * with a `too-large` error and the widget loses the whole result — display
 * names included, not just the pictures.
 */

/**
 * Square size, in pixels, avatars are re-encoded to. Canvas avatars render
 * between 20px and 42px, so this stays crisp on a 2x display.
 */
export const CANVAS_AVATAR_PIXEL_SIZE = DEFAULT_AVATAR_PIXEL_SIZE;

/** Per-avatar ceiling. A 96px WebP/JPEG avatar lands far under this. */
export const CANVAS_AVATAR_MAX_DATA_URL_LENGTH = 16 * 1_024;

/**
 * Combined ceiling across one response. Leaves ~24 KiB of the 64 KiB message
 * budget for names, pubkeys, and the envelope — ample for the 32-person
 * maximum a lookup can return.
 */
export const CANVAS_AVATAR_TOTAL_DATA_URL_BUDGET = 40 * 1_024;

/**
 * Fetches `avatarUrl` and re-encodes it small enough to embed in a canvas RPC
 * payload. Returns null when there is no avatar, the fetch fails, or the image
 * cannot be brought under {@link CANVAS_AVATAR_MAX_DATA_URL_LENGTH} — all of
 * which mean "render initials".
 *
 * Animated avatars collapse to their poster frame, and relay-hosted URLs go
 * through the media proxy, matching how the rest of the app resolves an avatar.
 */
export async function fetchCanvasAvatarDataUrl(
  avatarUrl: string | null,
): Promise<string | null> {
  const snapshotUrl = getAvatarSnapshotUrl(avatarUrl);
  if (!snapshotUrl) return null;
  const blob = await fetchAvatarBlob(rewriteRelayUrl(snapshotUrl));
  if (!blob) return null;
  return await downscaleAvatarDataUrl(blob, {
    maxDataUrlLength: CANVAS_AVATAR_MAX_DATA_URL_LENGTH,
    pixelSize: CANVAS_AVATAR_PIXEL_SIZE,
  });
}

/**
 * Drops avatars that would push a response past `totalBudget`, preserving
 * order: earlier entries keep their image and later ones degrade to initials.
 *
 * Returns a same-length array so callers can zip it back onto their rows. A
 * single avatar larger than the whole budget is dropped rather than allowed
 * through, so the ceiling always holds.
 */
export function selectAvatarsWithinBudget(
  dataUrls: ReadonlyArray<string | null>,
  totalBudget: number = CANVAS_AVATAR_TOTAL_DATA_URL_BUDGET,
): Array<string | null> {
  let spent = 0;
  return dataUrls.map((dataUrl) => {
    if (!dataUrl) return null;
    if (spent + dataUrl.length > totalBudget) return null;
    spent += dataUrl.length;
    return dataUrl;
  });
}
