#pragma once

#include <string>
#include <vector>
#include <map>
#include <sstream>
#include <iostream>

namespace chocobase {

class PostgrestQuery {
private:
    std::string base_url;
    std::string table_name;
    std::map<std::string, std::string> headers;
    std::map<std::string, std::string> params;

public:
    PostgrestQuery(std::string url, std::string table, std::map<std::string, std::string> h)
        : base_url(std::move(url)), table_name(std::move(table)), headers(std::move(h)) {}

    PostgrestQuery& select(const std::string& columns = "*") {
        params["select"] = columns;
        return *this;
    }

    PostgrestQuery& eq(const std::string& column, const std::string& value) {
        params[column] = "eq." + value;
        return *this;
    }

    PostgrestQuery& limit(int count) {
        params["limit"] = std::to_string(count);
        return *this;
    }

    std::string build_url() const {
        std::ostringstream ss;
        ss << base_url << "/rest/v1/" << table_name;
        if (!params.empty()) {
            ss << "?";
            bool first = true;
            for (const auto& [k, v] : params) {
                if (!first) ss << "&";
                ss << k << "=" << v;
                first = false;
            }
        }
        return ss.str();
    }
};

class StorageBucket {
private:
    std::string base_url;
    std::string bucket_name;
    std::map<std::string, std::string> headers;

public:
    StorageBucket(std::string url, std::string bucket, std::map<std::string, std::string> h)
        : base_url(std::move(url)), bucket_name(std::move(bucket)), headers(std::move(h)) {}

    std::string get_signed_url_endpoint(const std::string& path) const {
        return base_url + "/v1/storage/v1/object/sign/" + bucket_name + "/" + path;
    }
};

class StorageClient {
private:
    std::string base_url;
    std::map<std::string, std::string> headers;

public:
    StorageClient(std::string url, std::map<std::string, std::string> h)
        : base_url(std::move(url)), headers(std::move(h)) {}

    StorageBucket from(const std::string& bucket) {
        return StorageBucket(base_url, bucket, headers);
    }
};

class FunctionsClient {
private:
    std::string base_url;
    std::map<std::string, std::string> headers;

public:
    FunctionsClient(std::string url, std::map<std::string, std::string> h)
        : base_url(std::move(url)), headers(std::move(h)) {}

    std::string get_invocation_url(const std::string& function_name) const {
        return base_url + "/v1/functions/v1/" + function_name;
    }
};

class Client {
private:
    std::string url;
    std::string api_key;
    std::map<std::string, std::string> headers;

public:
    StorageClient storage;
    FunctionsClient functions;

    Client(std::string u, std::string key, std::map<std::string, std::string> custom_headers = {})
        : url(std::move(u)), api_key(std::move(key)),
          storage(url, headers), functions(url, headers) {
        if (!url.empty() && url.back() == '/') {
            url.pop_back();
        }
        headers["apikey"] = api_key;
        headers["Authorization"] = "Bearer " + api_key;
        headers["Content-Type"] = "application/json";
        for (auto& [k, v] : custom_headers) {
            headers[k] = v;
        }
        storage = StorageClient(url, headers);
        functions = FunctionsClient(url, headers);
    }

    PostgrestQuery from(const std::string& table) {
        return PostgrestQuery(url, table, headers);
    }

    const std::string& get_url() const { return url; }
    const std::string& get_api_key() const { return api_key; }
    const std::map<std::string, std::string>& get_headers() const { return headers; }
};

inline Client create_client(const std::string& url, const std::string& api_key, std::map<std::string, std::string> custom_headers = {}) {
    return Client(url, api_key, std::move(custom_headers));
}

} // namespace chocobase
