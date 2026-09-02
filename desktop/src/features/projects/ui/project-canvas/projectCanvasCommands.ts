import { invokeTauri } from "@/shared/api/tauri";

import {
  parseProjectCanvasPackageDescriptor,
  parseProjectCanvasPackageDescriptorForE2e,
  parseProjectCanvasPendingUpdates,
  type ProjectCanvasPackageDescriptor,
  type ProjectCanvasPendingUpdates,
} from "./projectCanvasProtocol";

/** The Tauri command surface shared by the Canvas host and its frame. */

export type ProjectCanvasPackageRequest = {
  communityId: string;
  projectId: string;
};

const parsePackageDescriptor =
  import.meta.env.MODE === "e2e"
    ? parseProjectCanvasPackageDescriptorForE2e
    : parseProjectCanvasPackageDescriptor;

export async function requestProjectCanvasPackage(
  command: "activate_project_canvas_package" | "get_project_canvas_package",
  request: ProjectCanvasPackageRequest,
): Promise<ProjectCanvasPackageDescriptor> {
  const response = await invokeTauri<unknown>(command, { request });
  return parsePackageDescriptor(response);
}

export async function releaseProjectCanvasPackage(
  loadId: string,
): Promise<void> {
  await invokeTauri("release_project_canvas_package", { loadId });
}

export async function commitProjectCanvasPackage(
  loadId: string,
): Promise<void> {
  await invokeTauri("commit_project_canvas_package", { loadId });
}

export async function requestProjectCanvasUpdates(
  request: ProjectCanvasPackageRequest,
): Promise<ProjectCanvasPendingUpdates> {
  const response = await invokeTauri<unknown>("get_project_canvas_updates", {
    request,
  });
  return parseProjectCanvasPendingUpdates(response);
}

export async function openProjectCanvasSource(
  request: ProjectCanvasPackageRequest,
): Promise<void> {
  await invokeTauri("open_project_canvas_source", { request });
}

export function projectCanvasErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "Canvas package failed.";
}
