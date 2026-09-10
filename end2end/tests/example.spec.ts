import { test, expect } from "@playwright/test";

test("unauthenticated visitors land on the sign-in page", async ({ page }) => {
  await page.goto("http://localhost:3000/");

  // The dashboard is behind auth, so `/` redirects to the sign-in screen.
  await expect(page).toHaveTitle("Sign in · finAll");
  await expect(page.locator("h1")).toHaveText("Sign in");
});
