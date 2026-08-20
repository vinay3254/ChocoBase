module ChocoBase
  class StorageBucket
    def initialize(base_url, bucket, headers)
      @base_url = base_url
      @bucket = bucket
      @headers = headers
    end

    def create_signed_url(path, expires_in: 3600)
      uri = URI("#{@base_url}/v1/storage/v1/object/sign/#{@bucket}/#{path}")
      http = Net::HTTP.new(uri.host, uri.port)
      http.use_ssl = (uri.scheme == "https")

      req = Net::HTTP::Post.new(uri.request_uri)
      @headers.each { |k, v| req[k] = v }
      req.body = { expires_in: expires_in }.to_json

      res = http.request(req)
      parsed = JSON.parse(res.body) rescue {}
      parsed["signed_url"] ? "#{@base_url}#{parsed['signed_url']}" : nil
    end
  end

  class Storage
    def initialize(base_url, headers)
      @base_url = base_url
      @headers = headers
    end

    def from(bucket)
      StorageBucket.new(@base_url, bucket, @headers)
    end
  end
end
