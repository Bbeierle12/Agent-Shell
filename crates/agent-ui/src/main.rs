use leptos::prelude::*;
use leptos::task::spawn_local;
use web_sys::js_sys;

mod api;

use api::SessionInfo;

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

// ── Types ───────────────────────────────────────────────────────────────

/// A chat message displayed in the UI.
#[derive(Clone, Debug)]
enum ChatItem {
    UserMessage(String),
    AssistantMessage(String),
    ToolCall(ToolCardState),
}

/// State for a tool execution card.
#[derive(Clone, Debug)]
struct ToolCardState {
    name: String,
    status: ToolStatus,
    output: Option<String>,
    is_error: bool,
    collapsed: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum ToolStatus {
    Running,
    Complete,
    Error,
}

/// Connection status.
#[derive(Clone, Debug, PartialEq)]
enum ConnStatus {
    Checking,
    Connected,
    Disconnected,
}

/// Main content view mode.
#[derive(Clone, Debug, PartialEq)]
enum ViewMode {
    Chat,
    Analytics,
}

/// Convert history messages from the server into ChatItems.
fn history_to_chat_items(messages: &[api::HistoryMessage]) -> Vec<ChatItem> {
    let mut items = Vec::new();
    for msg in messages {
        match msg.role.as_str() {
            "user" => items.push(ChatItem::UserMessage(msg.content.clone())),
            "assistant" => {
                // If the assistant message has tool_calls, show them.
                if let Some(tcs) = &msg.tool_calls {
                    if !msg.content.is_empty() {
                        items.push(ChatItem::AssistantMessage(msg.content.clone()));
                    }
                    for tc in tcs {
                        items.push(ChatItem::ToolCall(ToolCardState {
                            name: tc.name.clone(),
                            status: ToolStatus::Complete,
                            output: None,
                            is_error: false,
                            collapsed: true,
                        }));
                    }
                } else {
                    items.push(ChatItem::AssistantMessage(msg.content.clone()));
                }
            }
            "tool" => {
                // Tool result — attach output to last running tool card.
                if let Some(last_tool) = items.iter_mut().rev().find(|m| {
                    matches!(
                        m,
                        ChatItem::ToolCall(ToolCardState {
                            output: None,
                            ..
                        })
                    )
                }) {
                    if let ChatItem::ToolCall(ref mut card) = last_tool {
                        card.output = Some(msg.content.clone());
                    }
                }
            }
            _ => {}
        }
    }
    items
}

/// Scroll the messages container to the bottom.
fn scroll_to_bottom() {
    if let Some(window) = web_sys::window() {
        if let Some(doc) = window.document() {
            if let Some(el) = doc.query_selector(".messages").ok().flatten() {
                el.set_scroll_top(el.scroll_height());
            }
        }
    }
}

// ── App Component ───────────────────────────────────────────────────────

#[component]
fn App() -> impl IntoView {
    let (messages, set_messages) = signal(Vec::<ChatItem>::new());
    let (input, set_input) = signal(String::new());
    let (is_streaming, set_streaming) = signal(false);
    let (sessions, set_sessions) = signal(Vec::<SessionInfo>::new());
    let (active_session, set_active_session) = signal(Option::<String>::None);
    let (conn_status, set_conn_status) = signal(ConnStatus::Checking);
    let (show_settings, set_show_settings) = signal(false);
    let (server_config, set_server_config) = signal(Option::<api::ServerConfig>::None);
    let (view_mode, set_view_mode) = signal(ViewMode::Chat);
    let api_base = api::detect_api_base();

    // Check backend health on mount and load config.
    {
        let base = api_base.clone();
        let set_conn_status = set_conn_status.clone();
        let set_sessions = set_sessions.clone();
        let set_active_session = set_active_session.clone();
        let set_messages = set_messages.clone();
        let set_server_config = set_server_config.clone();
        spawn_local(async move {
            match api::health_check(&base).await {
                Ok(_) => {
                    set_conn_status.set(ConnStatus::Connected);
                    // Load config.
                    if let Ok(cfg) = api::get_config(&base).await {
                        set_server_config.set(Some(cfg));
                    }
                    // Load sessions and history for the first one.
                    if let Ok(sess) = api::list_sessions(&base).await {
                        if let Some(first) = sess.first() {
                            let sid = first.id.clone();
                            set_active_session.set(Some(sid.clone()));
                            if let Ok(history) = api::get_session_messages(&base, &sid).await {
                                set_messages.set(history_to_chat_items(&history));
                            }
                        }
                        set_sessions.set(sess);
                    }
                }
                Err(_) => set_conn_status.set(ConnStatus::Disconnected),
            }
        });
    }

    // Send message handler.
    let on_send = {
        let api_base = api_base.clone();
        let set_messages = set_messages.clone();
        let set_streaming = set_streaming.clone();
        move || {
            let text = input.get_untracked().trim().to_string();
            if text.is_empty() || is_streaming.get_untracked() {
                return;
            }
            set_input.set(String::new());

            set_messages.update(|msgs| {
                msgs.push(ChatItem::UserMessage(text.clone()));
            });
            scroll_to_bottom();

            set_streaming.set(true);
            let base = api_base.clone();
            let set_messages = set_messages.clone();
            let set_streaming = set_streaming.clone();
            let session_id = active_session.get_untracked();

            spawn_local(async move {
                set_messages.update(|msgs| {
                    msgs.push(ChatItem::AssistantMessage(String::new()));
                });

                let result = api::stream_chat(&base, &text, session_id.as_deref(), {
                    let set_messages = set_messages.clone();
                    move |event| {
                        match event {
                            api::StreamEvent::Token(token) => {
                                set_messages.update(|msgs| {
                                    if let Some(last) = msgs.iter_mut().rev().find(|m| {
                                        matches!(m, ChatItem::AssistantMessage(_))
                                    }) {
                                        if let ChatItem::AssistantMessage(ref mut s) = last {
                                            s.push_str(&token);
                                        }
                                    }
                                });
                            }
                            api::StreamEvent::ToolStart(name) => {
                                set_messages.update(|msgs| {
                                    msgs.push(ChatItem::ToolCall(ToolCardState {
                                        name,
                                        status: ToolStatus::Running,
                                        output: None,
                                        is_error: false,
                                        collapsed: false,
                                    }));
                                });
                            }
                            api::StreamEvent::ToolResult {
                                content,
                                is_error,
                            } => {
                                set_messages.update(|msgs| {
                                    if let Some(last) = msgs.iter_mut().rev().find(|m| {
                                        matches!(
                                            m,
                                            ChatItem::ToolCall(ToolCardState {
                                                status: ToolStatus::Running,
                                                ..
                                            })
                                        )
                                    }) {
                                        if let ChatItem::ToolCall(ref mut card) = last {
                                            card.status = if is_error {
                                                ToolStatus::Error
                                            } else {
                                                ToolStatus::Complete
                                            };
                                            card.output = Some(content);
                                            card.is_error = is_error;
                                        }
                                    }
                                    msgs.push(ChatItem::AssistantMessage(String::new()));
                                });
                            }
                            api::StreamEvent::Done => {}
                            api::StreamEvent::Error(e) => {
                                set_messages.update(|msgs| {
                                    if let Some(last) = msgs.iter_mut().rev().find(|m| {
                                        matches!(m, ChatItem::AssistantMessage(_))
                                    }) {
                                        if let ChatItem::AssistantMessage(ref mut s) = last {
                                            if s.is_empty() {
                                                *s = format!("Error: {}", e);
                                            }
                                        }
                                    }
                                });
                            }
                        }
                        scroll_to_bottom();
                    }
                })
                .await;

                if let Err(e) = result {
                    set_messages.update(|msgs| {
                        if let Some(last) = msgs.iter_mut().rev().find(|m| {
                            matches!(m, ChatItem::AssistantMessage(_))
                        }) {
                            if let ChatItem::AssistantMessage(ref mut s) = last {
                                if s.is_empty() {
                                    *s = format!("Connection error: {}", e);
                                }
                            }
                        }
                    });
                }

                // Clean up empty trailing assistant messages.
                set_messages.update(|msgs| {
                    while let Some(ChatItem::AssistantMessage(s)) = msgs.last() {
                        if s.is_empty() {
                            msgs.pop();
                        } else {
                            break;
                        }
                    }
                });

                set_streaming.set(false);
                scroll_to_bottom();
            });
        }
    };

    // Session click handler — loads history.
    let on_session_click = {
        let api_base = api_base.clone();
        move |session_id: String| {
            set_active_session.set(Some(session_id.clone()));
            set_messages.set(Vec::new());
            let base = api_base.clone();
            let set_messages = set_messages.clone();
            spawn_local(async move {
                if let Ok(history) = api::get_session_messages(&base, &session_id).await {
                    set_messages.set(history_to_chat_items(&history));
                    scroll_to_bottom();
                }
            });
        }
    };

    // New session handler.
    let on_new_session = {
        let api_base = api_base.clone();
        move || {
            let base = api_base.clone();
            let set_sessions = set_sessions.clone();
            let set_active_session = set_active_session.clone();
            let set_messages = set_messages.clone();
            spawn_local(async move {
                let name = format!(
                    "session-{}",
                    js_sys::Date::new_0()
                        .to_iso_string()
                        .as_string()
                        .unwrap_or_default()
                );
                if let Ok(info) = api::create_session(&base, &name).await {
                    set_active_session.set(Some(info.id));
                    set_messages.set(Vec::new());
                    if let Ok(sess) = api::list_sessions(&base).await {
                        set_sessions.set(sess);
                    }
                }
            });
        }
    };

    view! {
        <div class="app-layout">
            <Sidebar
                sessions=sessions
                active_session=active_session
                conn_status=conn_status
                on_click=on_session_click
                on_new=on_new_session
                on_settings=move || set_show_settings.set(true)
            />
            <div class="chat-area">
                <ChatHeader
                    config=server_config
                    view_mode=view_mode
                    set_view_mode=set_view_mode
                />
                {move || {
                    if view_mode.get() == ViewMode::Analytics {
                        view! { <AnalyticsDashboard api_base=api_base.clone() /> }.into_any()
                    } else {
                        view! {
                            <MessageList messages=messages set_messages=set_messages />
                            <InputBar
                                input=input
                                set_input=set_input
                                is_streaming=is_streaming
                                on_send=on_send.clone()
                            />
                        }.into_any()
                    }
                }}
            </div>
        </div>
        {move || {
            if show_settings.get() {
                view! {
                    <SettingsModal
                        config=server_config
                        on_close=move || set_show_settings.set(false)
                    />
                }.into_any()
            } else {
                view! { <div></div> }.into_any()
            }
        }}
    }
}

// ── Chat Header ─────────────────────────────────────────────────────────

#[component]
fn ChatHeader(
    config: ReadSignal<Option<api::ServerConfig>>,
    view_mode: ReadSignal<ViewMode>,
    set_view_mode: WriteSignal<ViewMode>,
) -> impl IntoView {
    view! {
        <div class="chat-header">
            <span class="chat-title">"Agent Shell"</span>
            {move || {
                config.get().map(|c| {
                    view! {
                        <span class="header-model">{c.provider.model.clone()}</span>
                    }
                })
            }}
            <div class="header-tabs">
                <button
                    class:tab-btn=true
                    class:active=move || view_mode.get() == ViewMode::Chat
                    on:click=move |_| set_view_mode.set(ViewMode::Chat)
                >"Chat"</button>
                <button
                    class:tab-btn=true
                    class:active=move || view_mode.get() == ViewMode::Analytics
                    on:click=move |_| set_view_mode.set(ViewMode::Analytics)
                >"Analytics"</button>
            </div>
        </div>
    }
}

// ── Sidebar ─────────────────────────────────────────────────────────────

#[component]
fn Sidebar(
    sessions: ReadSignal<Vec<SessionInfo>>,
    active_session: ReadSignal<Option<String>>,
    conn_status: ReadSignal<ConnStatus>,
    on_click: impl Fn(String) + Send + Sync + 'static + Clone,
    on_new: impl Fn() + Send + Sync + 'static + Clone,
    on_settings: impl Fn() + Send + Sync + 'static + Clone,
) -> impl IntoView {
    view! {
        <div class="sidebar">
            <div class="sidebar-header">
                <h2>"Sessions"</h2>
                <div class="sidebar-actions">
                    <button class="icon-btn" title="Settings" on:click={
                        let on_settings = on_settings.clone();
                        move |_| on_settings()
                    }>"\u{2699}"</button>
                    <button class="new-btn" on:click={
                        let on_new = on_new.clone();
                        move |_| on_new()
                    }>"+ New"</button>
                </div>
            </div>
            <div class="session-list">
                <For
                    each=move || sessions.get()
                    key=|s| s.id.clone()
                    children={
                        let on_click = on_click.clone();
                        move |session: SessionInfo| {
                            let id = session.id.clone();
                            let id2 = session.id.clone();
                            let on_click = on_click.clone();
                            view! {
                                <div
                                    class:session-item=true
                                    class:active=move || active_session.get().as_deref() == Some(id.as_str())
                                    on:click={
                                        let on_click = on_click.clone();
                                        let id = id2.clone();
                                        move |_| on_click(id.clone())
                                    }
                                >
                                    <div class="session-name">{session.name.clone()}</div>
                                    <div class="session-meta">
                                        {format!("{} msgs", session.message_count)}
                                    </div>
                                </div>
                            }
                        }
                    }
                />
            </div>
            <div class="sidebar-status">
                <span
                    class:status-dot=true
                    class:connected=move || conn_status.get() == ConnStatus::Connected
                    class:disconnected=move || conn_status.get() == ConnStatus::Disconnected
                    class:checking=move || conn_status.get() == ConnStatus::Checking
                ></span>
                {move || match conn_status.get() {
                    ConnStatus::Connected => "Connected",
                    ConnStatus::Disconnected => "Disconnected",
                    ConnStatus::Checking => "Checking...",
                }}
            </div>
        </div>
    }
}

// ── Settings Modal ──────────────────────────────────────────────────────

#[component]
fn SettingsModal(
    config: ReadSignal<Option<api::ServerConfig>>,
    on_close: impl Fn() + Send + Sync + 'static + Clone,
) -> impl IntoView {
    let on_close2 = on_close.clone();
    view! {
        <div class="modal-overlay" on:click={
            let on_close = on_close.clone();
            move |_| on_close()
        }>
            <div class="modal" on:click=move |ev| ev.stop_propagation()>
                <div class="modal-header">
                    <h2>"Settings"</h2>
                    <button class="modal-close" on:click=move |_| on_close2()>"\u{00D7}"</button>
                </div>
                <div class="modal-body">
                    {move || match config.get() {
                        Some(c) => view! {
                            <div class="settings-grid">
                                <div class="settings-section">
                                    <h3>"Provider"</h3>
                                    <div class="setting-row">
                                        <span class="setting-label">"Model"</span>
                                        <span class="setting-value">{c.provider.model.clone()}</span>
                                    </div>
                                    <div class="setting-row">
                                        <span class="setting-label">"Endpoint"</span>
                                        <span class="setting-value code">{c.provider.api_base.clone()}</span>
                                    </div>
                                    <div class="setting-row">
                                        <span class="setting-label">"Max Tokens"</span>
                                        <span class="setting-value">{c.provider.max_tokens}</span>
                                    </div>
                                    <div class="setting-row">
                                        <span class="setting-label">"Temperature"</span>
                                        <span class="setting-value">{format!("{:.1}", c.provider.temperature)}</span>
                                    </div>
                                    <div class="setting-row">
                                        <span class="setting-label">"API Key"</span>
                                        <span class="setting-value">{if c.provider.has_api_key { "configured" } else { "not set" }}</span>
                                    </div>
                                </div>
                                <div class="settings-section">
                                    <h3>"Server"</h3>
                                    <div class="setting-row">
                                        <span class="setting-label">"Address"</span>
                                        <span class="setting-value code">{format!("{}:{}", c.server.host, c.server.port)}</span>
                                    </div>
                                    <div class="setting-row">
                                        <span class="setting-label">"Auth"</span>
                                        <span class="setting-value">{if c.server.has_auth_token { "enabled" } else { "disabled" }}</span>
                                    </div>
                                    <div class="setting-row">
                                        <span class="setting-label">"CORS"</span>
                                        <span class="setting-value">{if c.server.cors { "enabled" } else { "disabled" }}</span>
                                    </div>
                                </div>
                                <div class="settings-section">
                                    <h3>"Session"</h3>
                                    <div class="setting-row">
                                        <span class="setting-label">"Context Window"</span>
                                        <span class="setting-value">{format!("{} messages", c.session.max_history)}</span>
                                    </div>
                                    <div class="setting-row">
                                        <span class="setting-label">"Auto-save"</span>
                                        <span class="setting-value">{if c.session.auto_save { "on" } else { "off" }}</span>
                                    </div>
                                </div>
                                <div class="settings-section">
                                    <h3>"Sandbox"</h3>
                                    <div class="setting-row">
                                        <span class="setting-label">"Mode"</span>
                                        <span class="setting-value">{c.sandbox.mode.clone()}</span>
                                    </div>
                                    <div class="setting-row">
                                        <span class="setting-label">"Image"</span>
                                        <span class="setting-value code">{c.sandbox.docker_image.clone()}</span>
                                    </div>
                                    <div class="setting-row">
                                        <span class="setting-label">"Timeout"</span>
                                        <span class="setting-value">{format!("{}s", c.sandbox.timeout_secs)}</span>
                                    </div>
                                </div>
                                <div class="settings-section">
                                    <h3>"Tools"</h3>
                                    <div class="tools-list">
                                        {c.tools.iter().map(|t| view! {
                                            <span class="tool-badge">{t.clone()}</span>
                                        }).collect::<Vec<_>>()}
                                    </div>
                                </div>
                            </div>
                        }.into_any(),
                        None => view! {
                            <p style="color: var(--text-muted)">"Loading configuration..."</p>
                        }.into_any(),
                    }}
                </div>
            </div>
        </div>
    }
}

// ── Analytics Dashboard ─────────────────────────────────────────────────

#[component]
fn AnalyticsDashboard(api_base: String) -> impl IntoView {
    let (summary, set_summary) = signal(Option::<api::AnalyticsSummary>::None);
    let (report, set_report) = signal(Option::<String>::None);
    let (report_period, set_report_period) = signal("week".to_string());
    let (loading, set_loading) = signal(true);

    // Load summary on mount.
    {
        let base = api_base.clone();
        spawn_local(async move {
            if let Ok(s) = api::get_analytics_summary(&base).await {
                set_summary.set(Some(s));
            }
            set_loading.set(false);
        });
    }

    // Load report when period changes.
    {
        let base = api_base.clone();
        spawn_local(async move {
            if let Ok(r) = api::get_analytics_report(&base, "week").await {
                set_report.set(Some(r));
            }
        });
    }

    let load_report = {
        let base = api_base.clone();
        move |period: String| {
            set_report_period.set(period.clone());
            set_report.set(None);
            let base = base.clone();
            spawn_local(async move {
                if let Ok(r) = api::get_analytics_report(&base, &period).await {
                    set_report.set(Some(r));
                }
            });
        }
    };

    view! {
        <div class="analytics-dashboard">
            {move || {
                if loading.get() {
                    return view! { <div class="analytics-loading">"Loading analytics..."</div> }.into_any();
                }

                match summary.get() {
                    Some(s) => {
                        let avg_duration = s.average_session_duration_secs
                            .map(|secs| {
                                let m = secs / 60;
                                let h = m / 60;
                                if h > 0 { format!("{}h {}m", h, m % 60) } else { format!("{}m", m) }
                            })
                            .unwrap_or_else(|| "—".to_string());

                        let top_tools = s.top_tools.clone();

                        view! {
                            <div class="analytics-content">
                                <div class="stats-grid">
                                    <div class="stat-card">
                                        <div class="stat-value">{s.total_sessions}</div>
                                        <div class="stat-label">"Total Sessions"</div>
                                    </div>
                                    <div class="stat-card">
                                        <div class="stat-value">{s.active_days}</div>
                                        <div class="stat-label">"Active Days"</div>
                                    </div>
                                    <div class="stat-card">
                                        <div class="stat-value">{avg_duration}</div>
                                        <div class="stat-label">"Avg Session"</div>
                                    </div>
                                    <div class="stat-card">
                                        <div class="stat-value">{s.deep_work_sessions}</div>
                                        <div class="stat-label">"Deep Work"</div>
                                    </div>
                                </div>

                                {if let Some(today) = s.today.clone() {
                                    view! {
                                        <div class="today-section">
                                            <h3>"Today"</h3>
                                            <div class="today-stats">
                                                <span>{format!("{} sessions", today.sessions)}</span>
                                                <span>{format!("{} messages", today.messages)}</span>
                                                <span>{today.active_time.clone()}</span>
                                                <span>{format!("{} tool calls", today.tool_calls)}</span>
                                                {if today.tool_errors > 0 {
                                                    view! { <span class="error-text">{format!("{} errors", today.tool_errors)}</span> }.into_any()
                                                } else {
                                                    view! { <span></span> }.into_any()
                                                }}
                                            </div>
                                        </div>
                                    }.into_any()
                                } else {
                                    view! { <div></div> }.into_any()
                                }}

                                {if !top_tools.is_empty() {
                                    view! {
                                        <div class="top-tools-section">
                                            <h3>"Top Tools"</h3>
                                            <div class="tools-bar-chart">
                                                {top_tools.iter().map(|(name, count)| {
                                                    let max = top_tools.first().map(|(_, c)| *c).unwrap_or(1);
                                                    let pct = (*count as f64 / max as f64 * 100.0) as u32;
                                                    let name = name.clone();
                                                    let count = *count;
                                                    view! {
                                                        <div class="bar-row">
                                                            <span class="bar-label">{name}</span>
                                                            <div class="bar-track">
                                                                <div class="bar-fill" style=format!("width: {}%", pct)></div>
                                                            </div>
                                                            <span class="bar-count">{count}</span>
                                                        </div>
                                                    }
                                                }).collect::<Vec<_>>()}
                                            </div>
                                        </div>
                                    }.into_any()
                                } else {
                                    view! { <div></div> }.into_any()
                                }}

                                <div class="report-section">
                                    <div class="report-header">
                                        <h3>"Report"</h3>
                                        <div class="report-tabs">
                                            <button
                                                class:report-tab=true
                                                class:active=move || report_period.get() == "week"
                                                on:click={
                                                    let load_report = load_report.clone();
                                                    move |_| load_report("week".to_string())
                                                }
                                            >"Weekly"</button>
                                            <button
                                                class:report-tab=true
                                                class:active=move || report_period.get() == "month"
                                                on:click={
                                                    let load_report = load_report.clone();
                                                    move |_| load_report("month".to_string())
                                                }
                                            >"Monthly"</button>
                                        </div>
                                    </div>
                                    {move || match report.get() {
                                        Some(md) => {
                                            let html = api::markdown_to_html(&md);
                                            view! { <div class="report-body md-content" inner_html=html></div> }.into_any()
                                        }
                                        None => view! { <div class="report-body">"Loading report..."</div> }.into_any(),
                                    }}
                                </div>
                            </div>
                        }.into_any()
                    }
                    None => view! {
                        <div class="analytics-empty">"No analytics data available."</div>
                    }.into_any(),
                }
            }}
        </div>
    }
}

// ── Message List ────────────────────────────────────────────────────────

#[component]
fn MessageList(
    messages: ReadSignal<Vec<ChatItem>>,
    set_messages: WriteSignal<Vec<ChatItem>>,
) -> impl IntoView {
    view! {
        <div class="messages">
            {move || {
                let items = messages.get();
                if items.is_empty() {
                    view! {
                        <div class="empty-state">
                            <h3>"Agent Shell"</h3>
                            <p>"Type a message to start chatting."</p>
                        </div>
                    }.into_any()
                } else {
                    let views: Vec<_> = items.iter().enumerate().map(|(i, item)| {
                        match item {
                            ChatItem::UserMessage(text) => {
                                view! {
                                    <div class="message user">
                                        <div class="role-label">"You"</div>
                                        {text.clone()}
                                    </div>
                                }.into_any()
                            }
                            ChatItem::AssistantMessage(text) if text.is_empty() => {
                                view! {
                                    <div class="message assistant">
                                        <div class="role-label">"Assistant"</div>
                                        <span style="color: var(--text-muted)">"..."</span>
                                    </div>
                                }.into_any()
                            }
                            ChatItem::AssistantMessage(text) => {
                                let rendered = api::markdown_to_html(text);
                                view! {
                                    <div class="message assistant">
                                        <div class="role-label">"Assistant"</div>
                                        <div class="md-content" inner_html=rendered></div>
                                    </div>
                                }.into_any()
                            }
                            ChatItem::ToolCall(card) => {
                                let card = card.clone();
                                view! {
                                    <ToolCard card=card idx=i set_messages=set_messages />
                                }.into_any()
                            }
                        }
                    }).collect();
                    view! { <div>{views}</div> }.into_any()
                }
            }}
        </div>
    }
}

// ── Tool Card ───────────────────────────────────────────────────────────

#[component]
fn ToolCard(
    card: ToolCardState,
    idx: usize,
    set_messages: WriteSignal<Vec<ChatItem>>,
) -> impl IntoView {
    let status_class = match card.status {
        ToolStatus::Running => "running",
        ToolStatus::Complete => "success",
        ToolStatus::Error => "error",
    };
    let status_text = match card.status {
        ToolStatus::Running => "running...",
        ToolStatus::Complete => "done",
        ToolStatus::Error => "error",
    };
    let has_output = card.output.is_some();
    let output = card.output.clone().unwrap_or_default();
    let collapsed = card.collapsed;

    view! {
        <div class="tool-card">
            <div class="tool-card-header" on:click=move |_| {
                set_messages.update(|msgs| {
                    if let Some(ChatItem::ToolCall(ref mut c)) = msgs.get_mut(idx) {
                        c.collapsed = !c.collapsed;
                    }
                });
            }>
                <span class="tool-name">{card.name.clone()}</span>
                <span class=format!("tool-status {}", status_class)>{status_text}</span>
            </div>
            {if has_output {
                let truncated = if output.chars().count() > 500 {
                    let s: String = output.chars().take(500).collect();
                    format!("{}...", s)
                } else {
                    output
                };
                view! {
                    <div class="tool-card-body" class:collapsed=collapsed>
                        {truncated}
                    </div>
                }.into_any()
            } else {
                view! { <div></div> }.into_any()
            }}
        </div>
    }
}

// ── Input Bar ───────────────────────────────────────────────────────────

#[component]
fn InputBar(
    input: ReadSignal<String>,
    set_input: WriteSignal<String>,
    is_streaming: ReadSignal<bool>,
    on_send: impl Fn() + 'static + Clone,
) -> impl IntoView {
    let on_send2 = on_send.clone();
    view! {
        <div class="input-area">
            <div class="input-wrapper">
                <textarea
                    rows="1"
                    placeholder="Type a message..."
                    prop:value=move || input.get()
                    on:input=move |ev| {
                        set_input.set(event_target_value(&ev));
                    }
                    on:keydown={
                        let on_send = on_send.clone();
                        move |ev: web_sys::KeyboardEvent| {
                            if ev.key() == "Enter" && !ev.shift_key() {
                                ev.prevent_default();
                                on_send();
                            }
                        }
                    }
                />
                <button
                    class="send-btn"
                    disabled=move || is_streaming.get() || input.get().trim().is_empty()
                    on:click={
                        move |_| on_send2()
                    }
                >
                    {move || if is_streaming.get() { "..." } else { "Send" }}
                </button>
            </div>
        </div>
    }
}
