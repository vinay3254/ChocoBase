require "net/http"
require "json"
require "uri"

require_relative "chocobase/auth"
require_relative "chocobase/postgrest"
require_relative "chocobase/storage"
require_relative "chocobase/functions"

module ChocoBase
  class Client
    attr_reader :url, :api_key, :headers, :auth, :storage, :functions

    def initialize(url, api_key, custom_headers: {})
      @url = url.chomp("/")
      @api_key = api_key
      @headers = {
        "apikey" => api_key,
        "Authorization" => "Bearer #{api_key}",
        "Content-Type" => "application/json"
      }.merge(custom_headers)

      @auth = Auth.new(@url, @headers)
      @storage = Storage.new(@url, @headers)
      @functions = Functions.new(@url, @headers)
    end

    def from(table)
      Postgrest.new(@url, table, @headers)
    end
  end

  def self.create_client(url, api_key, custom_headers: {})
    Client.new(url, api_key, custom_headers: custom_headers)
  end
end
