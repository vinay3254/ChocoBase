import Foundation

public struct User: Codable {
    public let id: Int
    public let username: String
    public let role: String
}

public struct AuthResponse: Codable {
    public let accessToken: String?
    public let refreshToken: String?
    public let user: User?
    public let error: String?
    
    enum CodingKeys: String, CodingKey {
        case accessToken = "access_token"
        case refreshToken = "refresh_token"
        case user
        case error
    }
}

public class AuthClient {
    private let url: URL
    private let headers: [String: String]
    
    init(url: URL, headers: [String: String]) {
        self.url = url
        self.headers = headers
    }
    
    public func signUp(username: String, password: String) async throws -> AuthResponse {
        let endpoint = url.appendingPathComponent("v1/auth/signup")
        var request = URLRequest(url: endpoint)
        request.httpMethod = "POST"
        for (k, v) in headers {
            request.setValue(v, forHTTPHeaderField: k)
        }
        let body = ["username": username, "password": password]
        request.httpBody = try JSONSerialization.data(withJSONObject: body)
        
        let (data, _) = try await URLSession.shared.data(for: request)
        return try JSONDecoder().decode(AuthResponse.self, from: data)
    }
    
    public func signIn(username: String, password: String) async throws -> AuthResponse {
        let endpoint = url.appendingPathComponent("v1/auth/token")
        var request = URLRequest(url: endpoint)
        request.httpMethod = "POST"
        for (k, v) in headers {
            request.setValue(v, forHTTPHeaderField: k)
        }
        let body = ["username": username, "password": password]
        request.httpBody = try JSONSerialization.data(withJSONObject: body)
        
        let (data, _) = try await URLSession.shared.data(for: request)
        return try JSONDecoder().decode(AuthResponse.self, from: data)
    }
}
