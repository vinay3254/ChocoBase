import Foundation

public class FunctionsClient {
    private let baseUrl: URL
    private let headers: [String: String]
    
    init(baseUrl: URL, headers: [String: String]) {
        self.baseUrl = baseUrl
        self.headers = headers
    }
    
    public func invoke(_ functionName: String, body: [String: Any]? = nil) async throws -> [String: Any] {
        let endpoint = baseUrl.appendingPathComponent("v1/functions/v1/\(functionName)")
        var request = URLRequest(url: endpoint)
        request.httpMethod = "POST"
        for (k, v) in headers {
            request.setValue(v, forHTTPHeaderField: k)
        }
        if let b = body {
            request.httpBody = try JSONSerialization.data(withJSONObject: b)
        }
        
        let (data, _) = try await URLSession.shared.data(for: request)
        if let json = try JSONSerialization.jsonObject(with: data) as? [String: Any] {
            return json
        }
        return [:]
    }
}
