import Foundation

public class StorageBucket {
    private let baseUrl: URL
    private let bucket: String
    private let headers: [String: String]
    
    init(baseUrl: URL, bucket: String, headers: [String: String]) {
        self.baseUrl = baseUrl
        self.bucket = bucket
        self.headers = headers
    }
    
    public func createSignedUrl(path: String, expiresIn: Int = 3600) async throws -> URL? {
        let endpoint = baseUrl.appendingPathComponent("v1/storage/v1/object/sign/\(bucket)/\(path)")
        var request = URLRequest(url: endpoint)
        request.httpMethod = "POST"
        for (k, v) in headers {
            request.setValue(v, forHTTPHeaderField: k)
        }
        let body = ["expires_in": expiresIn]
        request.httpBody = try JSONSerialization.data(withJSONObject: body)
        
        let (data, _) = try await URLSession.shared.data(for: request)
        if let json = try JSONSerialization.jsonObject(with: data) as? [String: Any],
           let signedUrlString = json["signed_url"] as? String {
            return URL(string: "\(baseUrl.absoluteString)\(signedUrlString)")
        }
        return nil
    }
}

public class StorageClient {
    private let baseUrl: URL
    private let headers: [String: String]
    
    init(baseUrl: URL, headers: [String: String]) {
        self.baseUrl = baseUrl
        self.headers = headers
    }
    
    public func from(_ bucket: String) -> StorageBucket {
        return StorageBucket(baseUrl: baseUrl, bucket: bucket, headers: headers)
    }
}
