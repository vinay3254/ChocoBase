"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.QueryBuilder = void 0;
class QueryBuilder {
    url;
    apikey;
    token;
    table;
    selectCols = "*";
    filters = [];
    orderCol;
    orderAsc = true;
    limitCount;
    offsetCount;
    constructor(url, apikey, token, table) {
        this.url = url.replace(/\/$/, "");
        this.apikey = apikey;
        this.token = token;
        this.table = table;
    }
    select(columns = "*") {
        this.selectCols = columns;
        return this;
    }
    eq(column, value) {
        this.filters.push({ col: column, op: "eq", val: value });
        return this;
    }
    neq(column, value) {
        this.filters.push({ col: column, op: "neq", val: value });
        return this;
    }
    gt(column, value) {
        this.filters.push({ col: column, op: "gt", val: value });
        return this;
    }
    lt(column, value) {
        this.filters.push({ col: column, op: "lt", val: value });
        return this;
    }
    order(column, options) {
        this.orderCol = column;
        this.orderAsc = options?.ascending ?? true;
        return this;
    }
    limit(count) {
        this.limitCount = count;
        return this;
    }
    range(from, to) {
        this.offsetCount = from;
        this.limitCount = to - from + 1;
        return this;
    }
    buildQueryString() {
        const params = [];
        if (this.selectCols && this.selectCols !== "*") {
            params.push(`select=${encodeURIComponent(this.selectCols)}`);
        }
        for (const f of this.filters) {
            params.push(`${encodeURIComponent(f.col)}=${f.op}.${encodeURIComponent(String(f.val))}`);
        }
        if (this.orderCol) {
            params.push(`order=${encodeURIComponent(this.orderCol)}.${this.orderAsc ? "asc" : "desc"}`);
        }
        if (this.limitCount !== undefined) {
            params.push(`limit=${this.limitCount}`);
        }
        if (this.offsetCount !== undefined) {
            params.push(`offset=${this.offsetCount}`);
        }
        return params.length > 0 ? `?${params.join("&")}` : "";
    }
    getHeaders() {
        const headers = {
            "Content-Type": "application/json",
            apikey: this.apikey,
        };
        if (this.token) {
            headers["Authorization"] = `Bearer ${this.token}`;
        }
        return headers;
    }
    async get() {
        try {
            const qs = this.buildQueryString();
            const res = await fetch(`${this.url}/v1/rest/${this.table}${qs}`, {
                method: "GET",
                headers: this.getHeaders(),
            });
            const json = await res.json();
            if (!res.ok) {
                return { data: null, error: new Error(json.error || "Query failed") };
            }
            return { data: json, error: null };
        }
        catch (e) {
            return { data: null, error: e };
        }
    }
    async insert(record) {
        try {
            const res = await fetch(`${this.url}/v1/rest/${this.table}`, {
                method: "POST",
                headers: this.getHeaders(),
                body: JSON.stringify(record),
            });
            const json = await res.json();
            if (!res.ok) {
                return { data: null, error: new Error(json.error || "Insert failed") };
            }
            return { data: json, error: null };
        }
        catch (e) {
            return { data: null, error: e };
        }
    }
    async update(values) {
        try {
            const qs = this.buildQueryString();
            const res = await fetch(`${this.url}/v1/rest/${this.table}${qs}`, {
                method: "PATCH",
                headers: this.getHeaders(),
                body: JSON.stringify(values),
            });
            const json = await res.json();
            if (!res.ok) {
                return { data: null, error: new Error(json.error || "Update failed") };
            }
            return { data: json, error: null };
        }
        catch (e) {
            return { data: null, error: e };
        }
    }
    async delete() {
        try {
            const qs = this.buildQueryString();
            const res = await fetch(`${this.url}/v1/rest/${this.table}${qs}`, {
                method: "DELETE",
                headers: this.getHeaders(),
            });
            const json = await res.json();
            if (!res.ok) {
                return { data: null, error: new Error(json.error || "Delete failed") };
            }
            return { data: json, error: null };
        }
        catch (e) {
            return { data: null, error: e };
        }
    }
}
exports.QueryBuilder = QueryBuilder;
