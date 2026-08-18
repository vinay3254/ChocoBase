//! Embedded Admin Web Dashboard for ChocoBase.

pub const DASHBOARD_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>ChocoBase Studio & Dashboard</title>
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
    <link href="https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@400;500;600;700&family=Plus+Jakarta+Sans:wght@400;500;600;700;800&display=swap" rel="stylesheet">
    <style>
        :root {
            --bg-base: #0c0e14;
            --bg-surface: #141721;
            --bg-elevated: #1a1e2b;
            --bg-active: #23283a;
            --border-subtle: #252a3d;
            --border-strong: #323850;
            --text-main: #f0f2f8;
            --text-muted: #8b92aa;
            --primary: #6366f1;
            --primary-hover: #4f46e5;
            --accent-green: #10b981;
            --accent-amber: #f59e0b;
            --accent-rose: #f43f5e;
            --accent-cyan: #06b6d4;
            --font-sans: 'Plus Jakarta Sans', -apple-system, BlinkMacSystemFont, sans-serif;
            --font-mono: 'JetBrains Mono', monospace;
            --radius-md: 10px;
            --radius-lg: 14px;
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

        /* Top Header */
        header {
            background-color: var(--bg-surface);
            border-bottom: 1px solid var(--border-subtle);
            padding: 0.75rem 1.5rem;
            display: flex;
            align-items: center;
            justify-content: space-between;
            z-index: 10;
        }

        .brand {
            display: flex;
            align-items: center;
            gap: 0.75rem;
            font-size: 1.15rem;
            font-weight: 800;
            letter-spacing: -0.02em;
        }

        .brand-badge {
            background: linear-gradient(135deg, var(--primary), var(--accent-cyan));
            color: white;
            padding: 0.25rem 0.6rem;
            border-radius: 6px;
            font-size: 0.75rem;
            font-weight: 700;
            text-transform: uppercase;
        }

        .status-pill {
            display: flex;
            align-items: center;
            gap: 0.5rem;
            background: var(--bg-elevated);
            border: 1px solid var(--border-subtle);
            padding: 0.35rem 0.85rem;
            border-radius: 9999px;
            font-size: 0.8rem;
            font-weight: 500;
            color: var(--text-muted);
        }

        .status-dot {
            width: 8px;
            height: 8px;
            border-radius: 50%;
            background-color: var(--accent-green);
            box-shadow: 0 0 10px var(--accent-green);
        }

        /* Main Workspace Layout */
        .workspace {
            display: grid;
            grid-template-columns: 280px 1fr;
            flex: 1;
            overflow: hidden;
        }

        /* Left Sidebar: Schema Browser & Navigation */
        aside {
            background-color: var(--bg-surface);
            border-right: 1px solid var(--border-subtle);
            display: flex;
            flex-direction: column;
            overflow-y: auto;
        }

        .sidebar-section {
            padding: 1.25rem 1rem 0.5rem;
        }

        .section-title {
            font-size: 0.75rem;
            font-weight: 700;
            text-transform: uppercase;
            letter-spacing: 0.05em;
            color: var(--text-muted);
            margin-bottom: 0.75rem;
            padding-left: 0.5rem;
            display: flex;
            justify-content: space-between;
            align-items: center;
        }

        .table-list {
            list-style: none;
            display: flex;
            flex-direction: column;
            gap: 0.25rem;
        }

        .table-item {
            padding: 0.6rem 0.75rem;
            border-radius: var(--radius-md);
            font-size: 0.875rem;
            font-weight: 600;
            cursor: pointer;
            display: flex;
            align-items: center;
            justify-content: space-between;
            color: var(--text-muted);
            transition: all 0.15s ease;
        }

        .table-item:hover, .table-item.active {
            background-color: var(--bg-elevated);
            color: var(--text-main);
        }

        .table-item.active {
            border-left: 3px solid var(--primary);
        }

        /* Center Content Area */
        main {
            display: flex;
            flex-direction: column;
            overflow: hidden;
            background-color: var(--bg-base);
        }

        /* SQL Editor Panel */
        .editor-container {
            padding: 1.25rem 1.5rem 0.75rem;
            background: var(--bg-surface);
            border-bottom: 1px solid var(--border-subtle);
            display: flex;
            flex-direction: column;
            gap: 0.75rem;
        }

        .editor-header {
            display: flex;
            justify-content: space-between;
            align-items: center;
        }

        .editor-title {
            font-size: 0.9rem;
            font-weight: 700;
        }

        .btn-group {
            display: flex;
            gap: 0.5rem;
        }

        button {
            font-family: var(--font-sans);
            font-size: 0.85rem;
            font-weight: 600;
            padding: 0.5rem 1.1rem;
            border-radius: 8px;
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

        textarea {
            width: 100%;
            height: 110px;
            background-color: var(--bg-base);
            border: 1px solid var(--border-strong);
            border-radius: var(--radius-md);
            color: var(--text-main);
            font-family: var(--font-mono);
            font-size: 0.9rem;
            line-height: 1.5;
            padding: 0.75rem 1rem;
            resize: vertical;
            outline: none;
            transition: border-color 0.15s ease;
        }

        textarea:focus {
            border-color: var(--primary);
        }

        /* Results & Tabs Panel */
        .results-panel {
            flex: 1;
            display: flex;
            flex-direction: column;
            padding: 1.25rem 1.5rem;
            overflow: hidden;
        }

        .tabs {
            display: flex;
            gap: 1.5rem;
            border-bottom: 1px solid var(--border-subtle);
            margin-bottom: 1rem;
        }

        .tab-btn {
            background: none;
            border: none;
            padding: 0.5rem 0;
            color: var(--text-muted);
            font-weight: 600;
            font-size: 0.9rem;
            cursor: pointer;
            position: relative;
            border-radius: 0;
        }

        .tab-btn.active {
            color: var(--text-main);
        }

        .tab-btn.active::after {
            content: '';
            position: absolute;
            bottom: -1px;
            left: 0;
            right: 0;
            height: 2px;
            background-color: var(--primary);
        }

        .tab-content {
            flex: 1;
            overflow: auto;
            border-radius: var(--radius-md);
            background: var(--bg-surface);
            border: 1px solid var(--border-subtle);
            position: relative;
        }

        /* Data Grid Table */
        table {
            width: 100%;
            border-collapse: collapse;
            font-size: 0.85rem;
            text-align: left;
        }

        th {
            background-color: var(--bg-elevated);
            color: var(--text-muted);
            font-weight: 700;
            text-transform: uppercase;
            font-size: 0.75rem;
            letter-spacing: 0.05em;
            padding: 0.75rem 1rem;
            border-bottom: 1px solid var(--border-subtle);
            position: sticky;
            top: 0;
            z-index: 2;
        }

        td {
            padding: 0.75rem 1rem;
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
            font-size: 0.9rem;
            gap: 0.5rem;
        }

        .banner-alert {
            padding: 0.75rem 1rem;
            border-radius: 8px;
            margin-bottom: 0.75rem;
            font-size: 0.85rem;
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
    </style>
</head>
<body>
    <header>
        <div class="brand">
            <span>⚡ ChocoBase Studio</span>
            <span class="brand-badge">v0.1.0</span>
        </div>
        <div class="status-pill">
            <div class="status-dot"></div>
            <span id="server-status">Connecting...</span>
        </div>
    </header>

    <div class="workspace">
        <aside>
            <div class="sidebar-section">
                <div class="section-title">
                    <span>Tables & Schema</span>
                    <button class="btn-secondary" style="padding: 0.2rem 0.5rem; font-size: 0.7rem;" onclick="loadTables()">Refresh</button>
                </div>
                <ul class="table-list" id="table-list">
                    <li class="empty-state" style="padding: 1rem 0;">Loading tables...</li>
                </ul>
            </div>
            <div class="sidebar-section" style="margin-top: auto; border-top: 1px solid var(--border-subtle); padding-top: 1rem;">
                <div class="section-title">Engine Metrics</div>
                <div id="metrics-view" style="font-size: 0.8rem; color: var(--text-muted); font-family: var(--font-mono); line-height: 1.6;">
                    Page Count: --<br>
                    Pages Read: --<br>
                    Cached Pages: --
                </div>
            </div>
        </aside>

        <main>
            <div class="editor-container">
                <div class="editor-header">
                    <span class="editor-title">SQL Query Console</span>
                    <div class="btn-group">
                        <button class="btn-secondary" onclick="clearQuery()">Clear</button>
                        <button class="btn-primary" onclick="runQuery()">▶ Execute (Ctrl+Enter)</button>
                    </div>
                </div>
                <textarea id="sql-input" placeholder="SELECT * FROM users;" spellcheck="false"></textarea>
                <div id="alert-box" class="banner-alert"></div>
            </div>

            <div class="results-panel">
                <div class="tabs">
                    <button class="tab-btn active" id="tab-results-btn">Query Results</button>
                </div>
                <div class="tab-content" id="results-content">
                    <div class="empty-state">Run a query to view results</div>
                </div>
            </div>
        </main>
    </div>

    <script>
        const sqlInput = document.getElementById('sql-input');
        const alertBox = document.getElementById('alert-box');
        const tableList = document.getElementById('table-list');
        const resultsContent = document.getElementById('results-content');
        const serverStatus = document.getElementById('server-status');
        const metricsView = document.getElementById('metrics-view');

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
                    serverStatus.innerText = 'Connected: ' + data.engine;
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

        async function loadTables() {
            try {
                const res = await fetch('/v1/tables');
                const data = await res.json();
                tableList.innerHTML = '';
                if (data.tables.length === 0) {
                    tableList.innerHTML = '<li class="empty-state" style="padding: 1rem 0;">No tables found</li>';
                    return;
                }
                data.tables.forEach(table => {
                    const li = document.createElement('li');
                    li.className = 'table-item';
                    li.innerHTML = `<span>🗄️ ${table}</span>`;
                    li.onclick = () => {
                        sqlInput.value = `SELECT * FROM ${table} LIMIT 50;`;
                        runQuery();
                    };
                    tableList.appendChild(li);
                });
            } catch (err) {
                tableList.innerHTML = '<li class="empty-state">Failed to load tables</li>';
            }
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
                loadTables();
                loadMetrics();
            } catch (err) {
                alertBox.className = 'banner-alert banner-error';
                alertBox.innerText = `Network Error: ${err.message}`;
                alertBox.style.display = 'block';
            }
        }

        function renderResult(result) {
            if (result === 'Ok') {
                resultsContent.innerHTML = '<div class="empty-state">Query executed successfully (OK)</div>';
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
                        let text = val === null ? 'NULL' : typeof val === 'object' ? JSON.stringify(val) : val;
                        html += `<td>${text}</td>`;
                    });
                    html += '</tr>';
                });
                html += '</tbody></table>';
                resultsContent.innerHTML = html;
            }
        }

        checkHealth();
        loadTables();
        loadMetrics();
    </script>
</body>
</html>
"#;
