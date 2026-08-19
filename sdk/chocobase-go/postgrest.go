package chocobase

import "fmt"

// PostgrestClient manages database queries.
type PostgrestClient struct {
	BaseURL string
	APIKey  string
}

func newPostgrestClient(baseURL, apiKey string) *PostgrestClient {
	return &PostgrestClient{
		BaseURL: baseURL,
		APIKey:  apiKey,
	}
}

// From returns a QueryBuilder for the specified table.
func (p *PostgrestClient) From(table string) *QueryBuilder {
	return &QueryBuilder{
		BaseURL: fmt.Sprintf("%s/rest/v1/%s", p.BaseURL, table),
		Table:   table,
		APIKey:  p.APIKey,
		Params:  make(map[string]string),
	}
}

// QueryBuilder fluently constructs PostgREST queries.
type QueryBuilder struct {
	BaseURL string
	Table   string
	APIKey  string
	Params  map[string]string
}

func (q *QueryBuilder) Select(columns string) *QueryBuilder {
	q.Params["select"] = columns
	return q
}

func (q *QueryBuilder) Eq(column string, value interface{}) *QueryBuilder {
	q.Params[column] = fmt.Sprintf("eq.%v", value)
	return q
}

func (q *QueryBuilder) Neq(column string, value interface{}) *QueryBuilder {
	q.Params[column] = fmt.Sprintf("neq.%v", value)
	return q
}

func (q *QueryBuilder) Gt(column string, value interface{}) *QueryBuilder {
	q.Params[column] = fmt.Sprintf("gt.%v", value)
	return q
}

func (q *QueryBuilder) Lt(column string, value interface{}) *QueryBuilder {
	q.Params[column] = fmt.Sprintf("lt.%v", value)
	return q
}

func (q *QueryBuilder) Limit(count int) *QueryBuilder {
	q.Params["limit"] = fmt.Sprintf("%d", count)
	return q
}

// QueryResult represents standard query execution payload.
type QueryResult struct {
	Data  []map[string]interface{} `json:"data"`
	Count int                      `json:"count"`
	Error string                   `json:"error,omitempty"`
}

func (q *QueryBuilder) Execute() (*QueryResult, error) {
	return &QueryResult{
		Data:  make([]map[string]interface{}, 0),
		Count: 0,
	}, nil
}
