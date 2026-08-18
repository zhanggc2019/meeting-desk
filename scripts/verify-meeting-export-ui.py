from pathlib import Path

from playwright.sync_api import sync_playwright


OUTPUT_DIR = Path("output/ui-verification")


def assert_inside_viewport(page, selector: str) -> None:
    """确认关键界面元素完全位于当前桌面视口内。"""
    box = page.locator(selector).bounding_box()
    assert box is not None, f"missing element: {selector}"
    viewport = page.viewport_size
    assert viewport is not None
    assert box["x"] >= 0 and box["y"] >= 0
    assert box["x"] + box["width"] <= viewport["width"]
    assert box["y"] + box["height"] <= viewport["height"]


def main() -> None:
    """检查录音导出窗口和应用内播放器的布局与关键交互。"""
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(headless=True)
        page = browser.new_page(viewport={"width": 1360, "height": 840}, device_scale_factor=1)
        page.set_default_timeout(5_000)
        page.goto("http://127.0.0.1:4173/")
        page.wait_for_load_state("networkidle")
        page.get_by_role("button", name="录音记录").click()
        page.get_by_role("button", name="导出 产品交付节奏讨论").click()
        page.get_by_role("dialog", name="导出“产品交付节奏讨论”").wait_for()
        assert_inside_viewport(page, ".export-dialog")
        page.screenshot(path=OUTPUT_DIR / "export-dialog.png", full_page=True)

        page.locator(".export-dialog-close").click()
        page.get_by_role("button", name="试听 产品交付节奏讨论").click()
        page.get_by_role("region", name="正在试听 产品交付节奏讨论").wait_for()
        assert_inside_viewport(page, ".meeting-audio-dock .meeting-audio-player")
        page.screenshot(path=OUTPUT_DIR / "list-player.png", full_page=True)

        page.get_by_role("button", name="产品交付节奏讨论", exact=False).first.click()
        page.get_by_role("button", name="试听", exact=True).click()
        page.get_by_role("region", name="正在试听 产品交付节奏讨论.m4a").wait_for()
        assert_inside_viewport(page, ".detail-page .meeting-audio-player")
        page.screenshot(path=OUTPUT_DIR / "detail-player.png", full_page=True)
        browser.close()


if __name__ == "__main__":
    main()
