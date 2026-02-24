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

// ── App Component ───────────────────────────────────────────────────────

#[component]
fn App() -> impl IntoView {
    let (messages, set_messages) = signal(Vec::<ChatItem>::new());
    let (input, set_input) = signal(String::new());
    let (is_streaming, set_streaming) = signal(false);
    let (sessions, set_sessions) = signal(Vec::<SessionInfo>::new());
    let (active_session, set_active_session) = signal(Option::<String>::None);
    let (conn_status, set_conn_status) = signal(ConnStatus::Checking);
    let api_base = api::detect_api_base();

    // Check backend health on mount.
    {
        let base = api_base.clone();
        let set_conn_status = set_conn_status.clone();
        let set_sessions = set_sessions.clone();
        let set_active_session = set_active_session.clone();
        spawn_local(async move {
            match api::health_check(&base).await {
                Ok(_) => {
                    set_conn_status.set(ConnStatus::Connected);
                    if let Ok(sess) = api::list_sessions(&base).await {
                        if let Some(first) = sess.first() {
                            set_active_session.set(Some(first.id.clone()));
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

            set_streaming.set(true);
            let base = api_base.clone();
            let set_messages = set_messages.clone();
            let set_streaming = set_streaming.clone();

            spawn_local(async move {
                set_messages.update(|msgs| {
                    msgs.push(ChatItem::AssistantMessage(String::new()));
                });

                let result = api::stream_chat(&base, &text, {
                    let set_messages = set_messages.clone();
                    move |event| match event {
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
            });
        }
    };

    // Session click handler.
    let on_session_click = {
        move |session_id: String| {
            set_active_session.set(Some(session_id));
            set_messages.set(Vec::new());
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
            />
            <div class="chat-area">
                <div class="chat-header">"Agent Shell"</div>
                <MessageList messages=messages set_messages=set_messages />
                <InputBar
                    input=input
                    set_input=set_input
                    is_streaming=is_streaming
                    on_send=on_send
                />
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
) -> impl IntoView {
    view! {
        <div class="sidebar">
            <div class="sidebar-header">
                <h2>"Sessions"</h2>
                <button class="new-btn" on:click={
                    let on_new = on_new.clone();
                    move |_| on_new()
                }>"+ New"</button>
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
                                view! {
                                    <div class="message assistant">
                                        <div class="role-label">"Assistant"</div>
                                        {text.clone()}
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
                let truncated = if output.len() > 500 {
                    format!("{}...", &output[..500])
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
