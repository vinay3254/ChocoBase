//! Embedded Admin Web Studio for ChocoBase.

pub const DASHBOARD_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>ChocoBase Studio</title>
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
    <link href="https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@400;500;600;700&family=Plus+Jakarta+Sans:wght@400;500;600;700;800&display=swap" rel="stylesheet">
    <style>
        :root {
            --bg-base: #090a0f;
            --bg-surface: #11141d;
            --bg-elevated: #181d2a;
            --bg-active: #22293b;
            --border-subtle: #1e2436;
            --border-strong: #2c354f;
            --text-main: #f3f5f9;
            --text-muted: #848da6;
            --primary: #4f46e5;
            --primary-hover: #4338ca;
            --accent-green: #10b981;
            --accent-amber: #f59e0b;
            --accent-rose: #f43f5e;
            --accent-cyan: #06b6d4;
            --font-sans: 'Plus Jakarta Sans', -apple-system, BlinkMacSystemFont, sans-serif;
            --font-mono: 'JetBrains Mono', monospace;
            --radius-md: 8px;
            --radius-lg: 12px;
        }

        * {
            box-sizing: border-box;
            margin: 0;
            padding: 0;
        }

        body {
            background-color: var(--bg-base);
            color: var(--text-main);
            font-family: var(--font-sans);
            height: 100vh;
            display: flex;
            flex-direction: column;
            overflow: hidden;
        }

        header {
            background-color: var(--bg-surface);
            border-bottom: 1px solid var(--border-subtle);
            padding: 0.65rem 1.25rem;
            display: flex;
            align-items: center;
            justify-content: space-between;
            z-index: 10;
        }

        .brand {
            display: flex;
            align-items: center;
            gap: 0.6rem;
            font-size: 1.1rem;
            font-weight: 800;
        }

        .brand-badge {
            background: linear-gradient(135deg, var(--primary), var(--accent-cyan));
            color: white;
            padding: 0.2rem 0.5rem;
            border-radius: 4px;
            font-size: 0.7rem;
            font-weight: 700;
            text-transform: uppercase;
        }

        .header-meta {
            display: flex;
            align-items: center;
            gap: 1rem;
        }

        .status-pill {
            display: flex;
            align-items: center;
            gap: 0.4rem;
            background: var(--bg-elevated);
            border: 1px solid var(--border-subtle);
            padding: 0.3rem 0.75rem;
            border-radius: 9999px;
            font-size: 0.78rem;
            color: var(--text-muted);
        }

        .status-dot {
            width: 7px;
            height: 7px;
            border-radius: 50%;
            background-color: var(--accent-green);
            box-shadow: 0 0 8px var(--accent-green);
        }

        .workspace {
            display: grid;
            grid-template-columns: 240px 1fr;
            flex: 1;
            overflow: hidden;
        }

        aside {
            background-color: var(--bg-surface);
            border-right: 1px solid var(--border-subtle);
            display: flex;
            flex-direction: column;
            overflow-y: auto;
        }

        .nav-group {
            padding: 1rem 0.75rem 0.5rem;
        }

        .nav-title {
            font-size: 0.7rem;
            font-weight: 700;
            text-transform: uppercase;
            letter-spacing: 0.06em;
            color: var(--text-muted);
            margin-bottom: 0.5rem;
            padding-left: 0.5rem;
        }

        .nav-item {
            padding: 0.55rem 0.75rem;
            border-radius: var(--radius-md);
            font-size: 0.85rem;
            font-weight: 600;
            cursor: pointer;
            display: flex;
            align-items: center;
            gap: 0.6rem;
            color: var(--text-muted);
            transition: all 0.15s ease;
            margin-bottom: 0.2rem;
        }

        .nav-item:hover, .nav-item.active {
            background-color: var(--bg-elevated);
            color: var(--text-main);
        }

        .nav-item.active {
            border-left: 3px solid var(--primary);
        }

        main {
            display: flex;
            flex-direction: column;
            overflow: hidden;
            background-color: var(--bg-base);
        }

        .view-section {
            display: none;
            flex: 1;
            flex-direction: column;
            overflow: hidden;
        }

        .view-section.active {
            display: flex;
        }

        .editor-container {
            padding: 1rem 1.25rem 0.75rem;
            background: var(--bg-surface);
            border-bottom: 1px solid var(--border-subtle);
            display: flex;
            flex-direction: column;
            gap: 0.6rem;
        }

        .editor-header {
            display: flex;
            justify-content: space-between;
            align-items: center;
        }

        .editor-title {
            font-size: 0.85rem;
            font-weight: 700;
        }

        .btn-group {
            display: flex;
            gap: 0.5rem;
        }

        button {
            font-family: var(--font-sans);
            font-size: 0.82rem;
            font-weight: 600;
            padding: 0.45rem 0.9rem;
            border-radius: 6px;
            border: none;
            cursor: pointer;
            transition: all 0.15s ease;
            display: inline-flex;
            align-items: center;
            gap: 0.4rem;
        }

        .btn-primary {
            background-color: var(--primary);
            color: white;
        }

        .btn-primary:hover {
            background-color: var(--primary-hover);
        }

        .btn-secondary {
            background-color: var(--bg-elevated);
            color: var(--text-main);
            border: 1px solid var(--border-strong);
        }

        .btn-secondary:hover {
            background-color: var(--bg-active);
        }

        textarea, input, select {
            background-color: var(--bg-base);
            border: 1px solid var(--border-strong);
            border-radius: var(--radius-md);
            color: var(--text-main);
            font-family: var(--font-mono);
            font-size: 0.85rem;
            padding: 0.6rem 0.85rem;
            outline: none;
        }

        textarea:focus, input:focus, select:focus {
            border-color: var(--primary);
        }

        textarea {
            width: 100%;
            height: 95px;
            resize: vertical;
        }

        .results-panel {
            flex: 1;
            display: flex;
            flex-direction: column;
            padding: 1rem 1.25rem;
            overflow: hidden;
        }

        .tab-content {
            flex: 1;
            overflow: auto;
            border-radius: var(--radius-md);
            background: var(--bg-surface);
            border: 1px solid var(--border-subtle);
        }

        table {
            width: 100%;
            border-collapse: collapse;
            font-size: 0.82rem;
            text-align: left;
        }

        th {
            background-color: var(--bg-elevated);
            color: var(--text-muted);
            font-weight: 700;
            text-transform: uppercase;
            font-size: 0.72rem;
            padding: 0.65rem 0.9rem;
            border-bottom: 1px solid var(--border-subtle);
            position: sticky;
            top: 0;
            z-index: 2;
        }

        td {
            padding: 0.65rem 0.9rem;
            border-bottom: 1px solid var(--border-subtle);
            color: var(--text-main);
            font-family: var(--font-mono);
        }

        tr:hover td {
            background-color: var(--bg-elevated);
        }

        .empty-state {
            display: flex;
            flex-direction: column;
            align-items: center;
            justify-content: center;
            height: 100%;
            color: var(--text-muted);
            font-size: 0.85rem;
            gap: 0.4rem;
            padding: 2rem;
        }

        .banner-alert {
            padding: 0.6rem 0.85rem;
            border-radius: 6px;
            margin-bottom: 0.5rem;
            font-size: 0.8rem;
            font-weight: 500;
            display: none;
        }

        .banner-error {
            background-color: rgba(244, 63, 94, 0.15);
            border: 1px solid var(--accent-rose);
            color: #fecdd3;
        }

        .banner-success {
            background-color: rgba(16, 185, 129, 0.15);
            border: 1px solid var(--accent-green);
            color: #a7f3d0;
        }

        .feed-container {
            font-family: var(--font-mono);
            font-size: 0.8rem;
            padding: 0.75rem;
            display: flex;
            flex-direction: column;
            gap: 0.4rem;
        }

        .feed-item {
            background: var(--bg-elevated);
            border: 1px solid var(--border-subtle);
            border-radius: 6px;
            padding: 0.5rem 0.75rem;
        }
    </style>
</head>
<body>
    <header>
        <div class="brand">
            <span>⚡ ChocoBase Studio</span>
            <span class="brand-badge">Engine v0.1.0</span>
        </div>
        <div class="header-meta">
            <div class="status-pill">
                <div class="status-dot"></div>
                <span id="server-status">Connecting...</span>
            </div>
        </div>
    </header>

    <div class="workspace">
        <aside>
            <div class="nav-group">
                <div class="nav-title">Studio Workspaces</div>
                <div class="nav-item active" onclick="switchView('sql')">💻 SQL & Tables</div>
                <div class="nav-item" onclick="switchView('storage')">📦 Storage Explorer</div>
                <div class="nav-item" onclick="switchView('auth')">🔐 Auth & Users</div>
                <div class="nav-item" onclick="switchView('functions')">⚡ Functions</div>
                <div class="nav-item" onclick="switchView('realtime')">📡 Live Realtime</div>
            </div>

            <div class="nav-group" style="margin-top: auto; border-top: 1px solid var(--border-subtle);">
                <div class="nav-title">Database Stats</div>
                <div id="metrics-view" style="font-size: 0.75rem; color: var(--text-muted); font-family: var(--font-mono); line-height: 1.5; padding: 0.25rem 0.5rem;">
                    Page Count: --<br>
                    Pages Read: --<br>
                    Cached Pages: --
                </div>
            </div>
        </aside>

        <main>
            <!-- SQL & Tables Workspace -->
            <div id="view-sql" class="view-section active">
                <div class="editor-container">
                    <div class="editor-header">
                        <span class="editor-title">SQL Query Runner</span>
                        <div class="btn-group">
                            <button class="btn-secondary" onclick="clearQuery()">Clear</button>
                            <button class="btn-primary" onclick="runQuery()">▶ Execute (Ctrl+Enter)</button>
                        </div>
                    </div>
                    <textarea id="sql-input" placeholder="SELECT * FROM users;" spellcheck="false"></textarea>
                    <div id="alert-box" class="banner-alert"></div>
                </div>

                <div class="results-panel">
                    <div class="tab-content" id="results-content">
                        <div class="empty-state">Execute a query or click a table to view data</div>
                    </div>
                </div>
            </div>

            <!-- Storage Explorer Workspace -->
            <div id="view-storage" class="view-section">
                <div class="editor-container">
                    <div class="editor-header">
                        <span class="editor-title">S3-Compatible Object Storage</span>
                        <button class="btn-primary" onclick="loadStorageBuckets()">↻ Refresh Buckets</button>
                    </div>
                </div>
                <div class="results-panel">
                    <div class="tab-content" id="storage-content">
                        <div class="empty-state">Loading storage buckets...</div>
                    </div>
                </div>
            </div>

            <!-- Auth & Users Workspace -->
            <div id="view-auth" class="view-section">
                <div class="editor-container">
                    <div class="editor-header">
                        <span class="editor-title">Identity & User Management</span>
                        <button class="btn-primary" onclick="loadAuthUsers()">↻ Refresh Users</button>
                    </div>
                </div>
                <div class="results-panel">
                    <div class="tab-content" id="auth-content">
                        <div class="empty-state">Loading users...</div>
                    </div>
                </div>
            </div>

            <!-- Serverless Functions Workspace -->
            <div id="view-functions" class="view-section">
                <div class="editor-container">
                    <div class="editor-header">
                        <span class="editor-title">Serverless Edge Functions</span>
                        <button class="btn-primary" onclick="loadFunctions()">↻ Refresh Registry</button>
                    </div>
                </div>
                <div class="results-panel">
                    <div class="tab-content" id="functions-content">
                        <div class="empty-state">Loading serverless functions...</div>
                    </div>
                </div>
            </div>

            <!-- Realtime Inspector Workspace -->
            <div id="view-realtime" class="view-section">
                <div class="editor-container">
                    <div class="editor-header">
                        <span class="editor-title">Live Realtime SSE Stream & Changefeed</span>
                        <div class="btn-group">
                            <input id="realtime-channel" value="general" style="width: 140px;" placeholder="Channel topic">
                            <button class="btn-primary" id="sse-toggle-btn" onclick="toggleRealtimeStream()">⚡ Connect SSE</button>
                            <button class="btn-secondary" onclick="clearRealtimeLogs()">Clear Logs</button>
                        </div>
                    </div>
                </div>
                <div class="results-panel">
                    <div class="tab-content" id="realtime-content">
                        <div id="realtime-logs" class="feed-container">
                            <div class="empty-state">Click Connect SSE to subscribe to live broadcast and database change events.</div>
                        </div>
                    </div>
                </div>
            </div>
        </main>
    </div>

    <script>
        const sqlInput = document.getElementById('sql-input');
        const alertBox = document.getElementById('alert-box');
        const resultsContent = document.getElementById('results-content');
        const serverStatus = document.getElementById('server-status');
        const metricsView = document.getElementById('metrics-view');
        let currentView = 'sql';
        let sseSource = null;

        function switchView(viewName) {
            currentView = viewName;
            document.querySelectorAll('.nav-item').forEach(el => el.classList.remove('active'));
            document.querySelectorAll('.view-section').forEach(el => el.classList.remove('active'));

            const targetSection = document.getElementById(`view-${viewName}`);
            if (targetSection) targetSection.classList.add('active');

            if (viewName === 'storage') loadStorageBuckets();
            else if (viewName === 'auth') loadAuthUsers();
            else if (viewName === 'functions') loadFunctions();
        }

        sqlInput.addEventListener('keydown', (e) => {
            if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
                runQuery();
            }
        });

        async function checkHealth() {
            try {
                const res = await fetch('/v1/health');
                const data = await res.json();
                if (data.status === 'healthy') {
                    serverStatus.innerText = 'Online: ' + data.engine;
                }
            } catch (err) {
                serverStatus.innerText = 'Disconnected';
            }
        }

        async function loadMetrics() {
            try {
                const res = await fetch('/v1/metrics');
                const stats = await res.json();
                metricsView.innerHTML = `Page Count: ${stats.page_count}<br>Pages Read: ${stats.pages_read}<br>Cached Pages: ${stats.cached_pages}`;
            } catch (e) {}
        }

        function clearQuery() {
            sqlInput.value = '';
            alertBox.style.display = 'none';
        }

        async function runQuery() {
            const sql = sqlInput.value.trim();
            if (!sql) return;

            alertBox.style.display = 'none';
            const startTime = performance.now();

            try {
                const res = await fetch('/v1/sql', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ sql })
                });
                const data = await res.json();
                const elapsed = (performance.now() - startTime).toFixed(1);

                if (!res.ok || data.status === 'error') {
                    alertBox.className = 'banner-alert banner-error';
                    alertBox.innerText = `Error: ${data.error}`;
                    alertBox.style.display = 'block';
                    resultsContent.innerHTML = `<div class="empty-state">Execution failed</div>`;
                    return;
                }

                alertBox.className = 'banner-alert banner-success';
                alertBox.innerText = `Executed in ${elapsed}ms`;
                alertBox.style.display = 'block';

                renderResult(data.result);
                loadMetrics();
            } catch (err) {
                alertBox.className = 'banner-alert banner-error';
                alertBox.innerText = `Network Error: ${err.message}`;
                alertBox.style.display = 'block';
            }
        }

        function renderResult(result) {
            if (result === 'Ok') {
                resultsContent.innerHTML = '<div class="empty-state">Statement executed successfully (OK)</div>';
            } else if (result && result.Modified !== undefined) {
                resultsContent.innerHTML = `<div class="empty-state">${result.Modified} row(s) modified</div>`;
            } else if (result && result.Rows) {
                const { columns, rows } = result.Rows;
                if (rows.length === 0) {
                    resultsContent.innerHTML = '<div class="empty-state">0 rows returned</div>';
                    return;
                }
                let html = '<table><thead><tr>';
                columns.forEach(col => {
                    html += `<th>${col}</th>`;
                });
                html += '</tr></thead><tbody>';
                rows.forEach(row => {
                    html += '<tr>';
                    row.forEach(val => {
                        let text = val === null ? '<span style="color: var(--accent-amber)">NULL</span>' : typeof val === 'object' ? JSON.stringify(val) : val;
                        html += `<td>${text}</td>`;
                    });
                    html += '</tr>';
                });
                html += '</tbody></table>';
                resultsContent.innerHTML = html;
            }
        }

        async function loadStorageBuckets() {
            const container = document.getElementById('storage-content');
            container.innerHTML = '<div class="empty-state">Loading buckets...</div>';
            try {
                const res = await fetch('/v1/storage/bucket');
                const buckets = await res.json();
                if (!Array.isArray(buckets) || buckets.length === 0) {
                    container.innerHTML = '<div class="empty-state">No storage buckets created. Create one via API or SQL.</div>';
                    return;
                }
                let html = '<table><thead><tr><th>Bucket Name</th><th>Public</th><th>Created At</th></tr></thead><tbody>';
                buckets.forEach(b => {
                    html += `<tr><td><strong>📁 ${b.name}</strong></td><td>${b.public ? '✅ Public' : '🔒 Private'}</td><td>${new Date(b.created_at * 1000).toLocaleString()}</td></tr>`;
                });
                html += '</tbody></table>';
                container.innerHTML = html;
            } catch (e) {
                container.innerHTML = `<div class="empty-state">Failed to load storage: ${e.message}</div>`;
            }
        }

        async function loadAuthUsers() {
            const container = document.getElementById('auth-content');
            container.innerHTML = '<div class="empty-state">Loading users...</div>';
            try {
                const res = await fetch('/v1/sql', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ sql: "SELECT id, username, role FROM _users LIMIT 100;" })
                });
                const data = await res.json();
                if (data.result && data.result.Rows) {
                    const { columns, rows } = data.result.Rows;
                    let html = '<table><thead><tr><th>ID</th><th>Username</th><th>Role</th></tr></thead><tbody>';
                    rows.forEach(r => {
                        html += `<tr><td>${r[0]}</td><td>👤 ${r[1]}</td><td><span class="brand-badge">${r[2]}</span></td></tr>`;
                    });
                    html += '</tbody></table>';
                    container.innerHTML = html;
                } else {
                    container.innerHTML = '<div class="empty-state">No users registered yet.</div>';
                }
            } catch (e) {
                container.innerHTML = `<div class="empty-state">Failed to load users: ${e.message}</div>`;
            }
        }

        async function loadFunctions() {
            const container = document.getElementById('functions-content');
            container.innerHTML = '<div class="empty-state">Active runtime: Subprocess isolated runner with timeout guard.</div>';
        }

        function toggleRealtimeStream() {
            const btn = document.getElementById('sse-toggle-btn');
            const logs = document.getElementById('realtime-logs');
            const channel = document.getElementById('realtime-channel').value.trim() || 'general';

            if (sseSource) {
                sseSource.close();
                sseSource = null;
                btn.innerText = '⚡ Connect SSE';
                btn.className = 'btn-primary';
                appendRealtimeLog('Disconnected from SSE stream.');
                return;
            }

            logs.innerHTML = '';
            appendRealtimeLog(`Connecting to SSE stream on channel '${channel}'...`);
            sseSource = new EventSource(`/v1/realtime/v1/stream?channel=${encodeURIComponent(channel)}`);

            sseSource.addEventListener('connected', (e) => {
                appendRealtimeLog(`[CONNECTED] ${e.data}`);
                btn.innerText = '⏹ Disconnect';
                btn.className = 'btn-secondary';
            });

            sseSource.addEventListener('broadcast', (e) => {
                appendRealtimeLog(`[BROADCAST] ${e.data}`);
            });

            sseSource.addEventListener('change', (e) => {
                appendRealtimeLog(`[DB CHANGE] ${e.data}`);
            });

            sseSource.onerror = () => {
                appendRealtimeLog('[ERROR] SSE connection error.');
            };
        }

        function appendRealtimeLog(msg) {
            const logs = document.getElementById('realtime-logs');
            const div = document.createElement('div');
            div.className = 'feed-item';
            div.innerText = `${new Date().toLocaleTimeString()} - ${msg}`;
            logs.prepend(div);
        }

        function clearRealtimeLogs() {
            document.getElementById('realtime-logs').innerHTML = '';
        }

        checkHealth();
        loadMetrics();
    </script>
</body>
</html>
"#;
