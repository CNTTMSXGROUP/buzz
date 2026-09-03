import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

test("sidebar có nút Não MSX, mở panel thấy cây thư mục", async ({ page }) => {
  await page.goto("/");
  const brainBtn = page.getByTestId("open-msx-brain-view");
  await expect(brainBtn).toBeVisible();
  await brainBtn.click();
  await expect(page.getByText("1. Thu Thập")).toBeVisible();
  await expect(page.getByText("2. Tinh Lọc")).toBeVisible();
});

async function openBrain(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("open-msx-brain-view").click();
}

test("click folder mở rộng, click file .md hiện nội dung", async ({ page }) => {
  await openBrain(page);
  await expect(page.getByText("2. Tinh Lọc")).toBeVisible();
  await page.getByText("2. Tinh Lọc").click(); // expand
  await expect(page.getByText("demo.md")).toBeVisible();
  await page.getByText("demo.md").click();
  await expect(page.getByText("nội dung demo não")).toBeVisible();
});

test("nút Quản trị mở bảng phân quyền", async ({ page }) => {
  await openBrain(page);
  await page.getByTestId("msx-admin-button").click();
  await expect(page.getByText("Quản trị phân quyền Não")).toBeVisible();
  await expect(page.getByText("E2E Owner")).toBeVisible();
});

test("nút Gửi ra kênh mở dropdown có ô ghi chú", async ({ page }) => {
  await openBrain(page);
  await page.getByText("2. Tinh Lọc").click();
  await page.getByText("demo.md").click();
  await page.getByTestId("msx-share-button").click();
  await expect(page.getByPlaceholder(/Ghi chú kèm tài liệu/)).toBeVisible();
});

test("nút 🧠 trong composer mở picker não, chọn file chèn token [nao:]", async ({ page }) => {
  await page.goto("/");
  // vào kênh general (starter) để có composer
  await page.getByText("general", { exact: true }).first().click();
  const attachBtn = page.getByRole("button", { name: "Đính kèm tài liệu từ Não MSX" });
  await expect(attachBtn).toBeVisible();
  await attachBtn.click();
  await expect(page.getByText("Não MSX — chọn tài liệu")).toBeVisible();
  await page.getByText("2. Tinh Lọc").click();
  await page.getByText("demo.md").click();
  const composer = page.getByTestId("message-composer").locator(".ProseMirror");
  await expect(composer).toContainText("[nao:2. Tinh Lọc/demo.md]");
});

test("chuột phải tên Não MSX hiện input đổi tên", async ({ page }) => {
  await openBrain(page);
  const title = page.locator("span.select-none", { hasText: "Não MSX" });
  await expect(title).toBeVisible();
  await title.dispatchEvent("contextmenu");
  const input = page.locator('input[placeholder], form input').first();
  await expect(input).toBeVisible();
  await input.fill("Não Công Ty");
  await input.press("Enter");
  await expect(page.getByText("Não Công Ty")).toBeVisible();
});
