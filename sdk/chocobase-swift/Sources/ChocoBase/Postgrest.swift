import Foundation

public class QueryBuilder {
    private let baseUrl: URL
    private let table: String
    private let headers: [String: String]
    private var params: [String: String] = [:]
    
    init(baseUrl: URL, table: String, headers: [String: String]) {
        self.baseUrl = baseUrl
        self.table = table
        self.headers = headers
    }
    
    public func select(_ columns: String = "*") -> QueryBuilder {
        params["select"] = columns
        return self
    }
    
    public func eq(_ column: String, value: CustomStringConvertible) -> QueryBuilder {
        params[column] = "eq.\(value)"
        return self
    }
    
    public func limit(_ count: Int) -> QueryBuilder {
        params["limit"] = String(count)
        return self
    }
    
    public func execute() async throws -> [[String: Any]] {
        var components = URLComponents(url: baseUrl.appendingPathComponent("rest/v1/\(table)"), resolvingAgainstBaseURL: true)
        components?.queryItems = params.map { URLQueryItem(name: $0.key, value: $0.value) }
        guard let targetUrl = components?.url else { return [] }
        
        var request = URLRequest(url: targetUrl)
        request.httpMethod = "GET"
        for (k, v) in headers {
            request.setValue(v, forHTTPHeaderField: k)
        }
        
        let (data, _) = try await URLSession.shared.data(for: request)
        let json = try JSONSerialization.jsonObject(with: data)
        if let list = json as? [[String: Any]] {
            return list
        } else if let dict = json as? [String: Any], let rows = dict["rows"] as? [[String: Any]] {
            return rows
        }
        return []
    }
}
