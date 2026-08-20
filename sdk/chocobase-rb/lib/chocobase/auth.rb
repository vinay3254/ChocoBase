module ChocoBase
  class Auth
    def initialize(base_url, headers)
      @base_url = base_url
      @headers = headers
    end

    def sign_up(username, password)
      post("/v1/auth/signup", { username: username, password: password })
    end

    def sign_in(username, password)
      post("/v1/auth/token", { username: username, password: password })
    end

    private

    def post(path, body)
      uri = URI("#{@base_url}#{path}")
      http = Net::HTTP.new(uri.host, uri.port)
      http.use_ssl = (uri.scheme == "https")

      req = Net::HTTP::Post.new(uri.request_uri)
      @headers.each { |k, v| req[k] = v }
      req.body = body.to_json

      res = http.request(req)
      JSON.parse(res.body) rescue {}
    end
  end
end
