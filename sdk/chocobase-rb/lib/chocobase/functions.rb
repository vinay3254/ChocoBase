module ChocoBase
  class Functions
    def initialize(base_url, headers)
      @base_url = base_url
      @headers = headers
    end

    def invoke(function_name, body = {})
      uri = URI("#{@base_url}/v1/functions/v1/#{function_name}")
      http = Net::HTTP.new(uri.host, uri.port)
      http.use_ssl = (uri.scheme == "https")

      req = Net::HTTP::Post.new(uri.request_uri)
      @headers.each { |k, v| req[k] = v }
      req.body = body.to_json

      res = http.request(req)
      JSON.parse(res.body) rescue { "raw" => res.body }
    end
  end
end
