module ChocoBase
  class Postgrest
    def initialize(base_url, table, headers)
      @base_url = base_url
      @table = table
      @headers = headers
      @params = {}
    end

    def select(columns = "*")
      @params["select"] = columns
      self
    end

    def eq(column, value)
      @params[column] = "eq.#{value}"
      self
    end

    def limit(count)
      @params["limit"] = count.to_s
      self
    end

    def execute
      uri = URI("#{@base_url}/rest/v1/#{@table}")
      uri.query = URI.encode_www_form(@params) unless @params.empty?

      http = Net::HTTP.new(uri.host, uri.port)
      http.use_ssl = (uri.scheme == "https")

      req = Net::HTTP::Get.new(uri.request_uri)
      @headers.each { |k, v| req[k] = v }

      res = http.request(req)
      parsed = JSON.parse(res.body) rescue []
      parsed.is_a?(Hash) && parsed.key?("rows") ? parsed["rows"] : parsed
    end
  end
end
