use headless_chrome::{Browser, LaunchOptions, Tab};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, OwnedSemaphorePermit, RwLock, Semaphore};
use tokio::time::timeout;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::models::{OutputFormat, PageExtractResult, PageMetadata};

const IDLE_TAB_TIMEOUT_SECS: u64 = 1;
const BROWSER_TIMEOUT_SECS: u64 = 10;

struct IdleTab {
    id: Uuid,
    tab: Arc<Tab>,
}

/// Browser lifecycle manager with tab pooling and auto-restart
pub struct BrowserManager {
    browser: RwLock<Arc<Browser>>,
    semaphore: Arc<Semaphore>,
    idle_tabs: Arc<Mutex<Vec<IdleTab>>>,
    max_concurrent_tabs: usize,
}

/// RAII guard for tab cleanup
pub struct TabGuard {
    tab: Arc<Tab>,
    tab_id: Uuid,
    idle_tabs: Arc<Mutex<Vec<IdleTab>>>,
    _permit: OwnedSemaphorePermit,
}

impl TabGuard {
    pub fn tab(&self) -> &Arc<Tab> {
        &self.tab
    }
}

impl Drop for TabGuard {
    fn drop(&mut self) {
        let tab = self.tab.clone();
        let tab_id = self.tab_id;
        let idle_tabs = self.idle_tabs.clone();

        tokio::spawn(async move {
            {
                let mut tabs = idle_tabs.lock().await;
                tabs.push(IdleTab { id: tab_id, tab: tab.clone() });
            }

            tokio::time::sleep(Duration::from_secs(IDLE_TAB_TIMEOUT_SECS)).await;

            let mut tabs = idle_tabs.lock().await;
            if let Some(pos) = tabs.iter().position(|t| t.id == tab_id) {
                let idle_tab = tabs.remove(pos);
                let _ = idle_tab.tab.close(true);
            }
        });
    }
}

impl BrowserManager {
    pub fn new(max_concurrent_tabs: usize) -> AppResult<Self> {
        let browser = Self::launch_browser()?;

        Ok(Self {
            browser: RwLock::new(Arc::new(browser)),
            semaphore: Arc::new(Semaphore::new(max_concurrent_tabs)),
            idle_tabs: Arc::new(Mutex::new(Vec::new())),
            max_concurrent_tabs,
        })
    }

    fn launch_browser() -> AppResult<Browser> {
        let launch_options = LaunchOptions::default_builder()
            .headless(true)
            .sandbox(true)
            .idle_browser_timeout(Duration::from_secs(600))
            .build()
            .map_err(|e| AppError::Browser(format!("Launch options failed: {}", e)))?;

        Browser::new(launch_options)
            .map_err(|e| AppError::Browser(format!("Browser launch failed: {}", e)))
    }

    async fn restart_browser(&self) -> AppResult<()> {
        warn!("Restarting browser");

        {
            let mut idle_tabs = self.idle_tabs.lock().await;
            idle_tabs.clear();
        }

        let new_browser = Self::launch_browser()?;

        {
            let mut browser = self.browser.write().await;
            *browser = Arc::new(new_browser);
        }

        info!("Browser restarted");
        Ok(())
    }

    fn is_connection_error(error_msg: &str) -> bool {
        error_msg.contains("connection is closed")
            || error_msg.contains("connection closed")
            || error_msg.contains("not connected")
            || error_msg.contains("Browser has been closed")
    }

    async fn create_tab_with_retry(&self) -> AppResult<Arc<Tab>> {
        let browser = self.browser.read().await;
        match browser.new_tab() {
            Ok(tab) => return Ok(tab),
            Err(e) => {
                let error_msg = e.to_string();
                if Self::is_connection_error(&error_msg) {
                    drop(browser);
                } else {
                    return Err(AppError::Browser(format!("Tab creation failed: {}", e)));
                }
            }
        }

        self.restart_browser().await?;

        let browser = self.browser.read().await;
        browser
            .new_tab()
            .map_err(|e| AppError::Browser(format!("Tab creation failed after restart: {}", e)))
    }

    pub async fn acquire_tab(&self) -> AppResult<TabGuard> {
        let permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| AppError::Browser(format!("Semaphore error: {}", e)))?;

        // Try reusing idle tab
        {
            let mut idle_tabs = self.idle_tabs.lock().await;
            if let Some(idle_tab) = idle_tabs.pop() {
                if idle_tab.tab.get_target_info().is_ok() {
                    debug!(tab_id = %idle_tab.id, "Reusing tab");
                    return Ok(TabGuard {
                        tab: idle_tab.tab,
                        tab_id: idle_tab.id,
                        idle_tabs: self.idle_tabs.clone(),
                        _permit: permit,
                    });
                }
            }
        }

        let tab = self.create_tab_with_retry().await?;
        let tab_id = Uuid::new_v4();
        debug!(tab_id = %tab_id, "New tab created");

        Ok(TabGuard {
            tab,
            tab_id,
            idle_tabs: self.idle_tabs.clone(),
            _permit: permit,
        })
    }

    pub async fn scrape_page(
        &self,
        url: &str,
        output_format: OutputFormat,
    ) -> AppResult<(PageMetadata, String)> {
        let tab_guard = self.acquire_tab().await?;
        let tab = tab_guard.tab();

        let result = timeout(
            Duration::from_secs(BROWSER_TIMEOUT_SECS),
            self.do_scrape(tab, url, output_format),
        )
        .await;

        match result {
            Ok(inner_result) => inner_result,
            Err(_) => {
                error!(url, timeout = BROWSER_TIMEOUT_SECS, "Page load timeout");
                Err(AppError::Timeout(format!(
                    "Timeout after {}s: {}",
                    BROWSER_TIMEOUT_SECS, url
                )))
            }
        }
    }

    async fn do_scrape(
        &self,
        tab: &Arc<Tab>,
        url: &str,
        output_format: OutputFormat,
    ) -> AppResult<(PageMetadata, String)> {
        let tab_clone = tab.clone();
        let url_owned = url.to_string();

        tokio::task::spawn_blocking(move || {
            tab_clone
                .navigate_to(&url_owned)
                .map_err(|e| AppError::Browser(format!("Navigation failed: {}", e)))?;

            tab_clone
                .wait_until_navigated()
                .map_err(|e| AppError::Browser(format!("Navigation wait failed: {}", e)))?;

            tab_clone
                .wait_for_element_with_custom_timeout("body", Duration::from_secs(5))
                .map_err(|e| AppError::Browser(format!("Body element wait failed: {}", e)))?;

            Ok::<_, AppError>(())
        })
        .await
        .map_err(|e| AppError::Internal(format!("Task join error: {}", e)))??;

        let tab_clone = tab.clone();
        let extract_result: PageExtractResult = tokio::task::spawn_blocking(move || {
            let js_code = r#"
                JSON.stringify((() => {
                    const og = {};
                    const metaTags = document.querySelectorAll('meta[property^="og:"]');
                    for (let i = 0; i < metaTags.length; i++) {
                        const tag = metaTags[i];
                        const prop = tag.getAttribute('property');
                        const content = tag.getAttribute('content');
                        if (prop && content) {
                            og[prop] = content;
                        }
                    }
                    return {
                        title: document.title || '',
                        og_tags: og,
                        body_html: document.body ? document.body.outerHTML : '<body></body>'
                    };
                })())
            "#;

            let result = tab_clone
                .evaluate(js_code, false)
                .map_err(|e| AppError::Browser(format!("JS evaluation failed: {}", e)))?;

            let json_string = result
                .value
                .ok_or_else(|| AppError::Browser("No JS result".to_string()))?;

            let json_str = json_string
                .as_str()
                .ok_or_else(|| AppError::Browser("Invalid JS result type".to_string()))?;

            serde_json::from_str(json_str)
                .map_err(|e| AppError::Browser(format!("JS result parse failed: {}", e)))
        })
        .await
        .map_err(|e| AppError::Internal(format!("Task join error: {}", e)))??;

        let metadata = PageMetadata {
            title: extract_result.title,
            og_tags: extract_result.og_tags,
        };

        let content = match output_format {
            OutputFormat::Html => extract_result.body_html,
            OutputFormat::Markdown => htmd::convert(&extract_result.body_html)
                .map_err(|e| AppError::Internal(format!("Markdown conversion failed: {}", e)))?,
        };

        info!(url, title = %metadata.title, len = content.len(), "Scraped");

        Ok((metadata, content))
    }

    pub async fn stats(&self) -> BrowserStats {
        let idle_count = self.idle_tabs.lock().await.len();
        let available_permits = self.semaphore.available_permits();

        BrowserStats {
            max_concurrent: self.max_concurrent_tabs,
            available_slots: available_permits,
            idle_tabs: idle_count,
            active_tabs: self.max_concurrent_tabs - available_permits,
        }
    }
}

#[derive(Debug)]
pub struct BrowserStats {
    pub max_concurrent: usize,
    pub available_slots: usize,
    pub idle_tabs: usize,
    pub active_tabs: usize,
}
