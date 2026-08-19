export class PostgrestQueryBuilder {
    url;
    table;
    headers;
    params;
    method = 'GET';
    body;
    constructor(url, table, headers = {}) {
        this.url = url.replace(/\/+$/, '');
        this.table = table;
        this.headers = { ...headers };
        this.params = new URLSearchParams();
    }
    select(columns = '*') {
        this.method = 'GET';
        this.params.set('select', columns);
        return this;
    }
    insert(values) {
        this.method = 'POST';
        this.body = values;
        return this;
    }
    update(values) {
        this.method = 'PATCH';
        this.body = values;
        return this;
    }
    delete() {
        this.method = 'DELETE';
        return this;
    }
    eq(column, value) {
        this.params.set(column, `eq.${value}`);
        return this;
    }
    neq(column, value) {
        this.params.set(column, `neq.${value}`);
        return this;
    }
    gt(column, value) {
        this.params.set(column, `gt.${value}`);
        return this;
    }
    gte(column, value) {
        this.params.set(column, `gte.${value}`);
        return this;
    }
    lt(column, value) {
        this.params.set(column, `lt.${value}`);
        return this;
    }
    lte(column, value) {
        this.params.set(column, `lte.${value}`);
        return this;
    }
    like(column, pattern) {
        this.params.set(column, `like.${pattern}`);
        return this;
    }
    ilike(column, pattern) {
        this.params.set(column, `ilike.${pattern}`);
        return this;
    }
    is(column, value) {
        if (value === null || value === 'null') {
            this.params.set(column, 'is.null');
        }
        else {
            this.params.set(column, 'is.not.null');
        }
        return this;
    }
    in(column, values) {
        this.params.set(column, `in.(${values.join(',')})`);
        return this;
    }
    order(column, options = { ascending: true }) {
        const dir = options.ascending ? 'asc' : 'desc';
        this.params.set('order', `${column}.${dir}`);
        return this;
    }
    limit(count) {
        this.params.set('limit', count.toString());
        return this;
    }
    range(from, to) {
        this.params.set('offset', from.toString());
        this.params.set('limit', (to - from + 1).toString());
        return this;
    }
    async execute() {
        const qs = this.params.toString();
        const endpoint = `${this.url}/v1/rest/${this.table}${qs ? `?${qs}` : ''}`;
        const init = {
            method: this.method,
            headers: {
                'Content-Type': 'application/json',
                ...this.headers,
            },
        };
        if (this.body && this.method !== 'GET') {
            init.body = JSON.stringify(this.body);
        }
        try {
            const res = await fetch(endpoint, init);
            const data = await res.json().catch(() => null);
            if (!res.ok) {
                return {
                    data: null,
                    error: {
                        message: data?.error || res.statusText,
                        code: data?.code,
                    },
                    status: res.status,
                    statusText: res.statusText,
                };
            }
            return {
                data: data,
                error: null,
                status: res.status,
                statusText: res.statusText,
            };
        }
        catch (err) {
            return {
                data: null,
                error: {
                    message: err.message || 'Network request failed',
                },
                status: 0,
                statusText: 'Network Error',
            };
        }
    }
    // Promise-like then for direct await support e.g. await client.from('t').select()
    then(onfulfilled, onrejected) {
        return this.execute().then(onfulfilled, onrejected);
    }
}
