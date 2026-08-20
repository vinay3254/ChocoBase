import Foundation

/// Official Swift Client for ChocoBase.
public class ChocoClient {
    public let url: URL
    public let apiKey: String
    public let headers: [String: String]
    
    public let auth: AuthClient
    public let storage: StorageClient
    public let functions: FunctionsClient
    
    public init(url: URL, apiKey: String, customHeaders: [String: String]? = nil) {
        self.url = url
        self.apiKey = apiKey
        var h = [
            "apikey": apiKey,
            "Authorization": "Bearer \(apiKey)",
            "Content-Type": "application/json"
        ]
        if let custom = customHeaders {
            for (k, v) in custom {
                h[k] = v
            }
        }
        self.headers = h
        self.auth = AuthClient(url: url, headers: h)
        self.storage = StorageClient(url: url, headers: h)
        self.functions = FunctionsClient(url: url, headers: h)
    }
    
    public func from(_ table: String) -> QueryBuilder {
        return QueryBuilder(baseUrl: url, table: table, headers: headers)
    }
}

public func createClient(url: URL, apiKey: String, customHeaders: [String: String]? = nil) -> ChocoClient {
    return ChocoClient(url: url, apiKey: apiKey, customHeaders: customHeaders)
}
