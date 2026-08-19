"use strict";
var __createBinding = (this && this.__createBinding) || (Object.create ? (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    var desc = Object.getOwnPropertyDescriptor(m, k);
    if (!desc || ("get" in desc ? !m.__esModule : desc.writable || desc.configurable)) {
      desc = { enumerable: true, get: function() { return m[k]; } };
    }
    Object.defineProperty(o, k2, desc);
}) : (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    o[k2] = m[k];
}));
var __exportStar = (this && this.__exportStar) || function(m, exports) {
    for (var p in m) if (p !== "default" && !Object.prototype.hasOwnProperty.call(exports, p)) __createBinding(exports, m, p);
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.ChocoBaseClient = void 0;
exports.createClient = createClient;
const auth_js_1 = require("./auth.js");
const functions_js_1 = require("./functions.js");
const graphql_js_1 = require("./graphql.js");
const postgrest_js_1 = require("./postgrest.js");
const realtime_js_1 = require("./realtime.js");
const storage_js_1 = require("./storage.js");
__exportStar(require("./auth.js"), exports);
__exportStar(require("./functions.js"), exports);
__exportStar(require("./graphql.js"), exports);
__exportStar(require("./postgrest.js"), exports);
__exportStar(require("./realtime.js"), exports);
__exportStar(require("./storage.js"), exports);
class ChocoBaseClient {
    auth;
    storage;
    functions;
    graphql;
    realtime;
    url;
    apikey;
    constructor(url, apikey, options) {
        this.url = url.replace(/\/$/, "");
        this.apikey = apikey;
        this.auth = new auth_js_1.AuthClient(this.url, this.apikey);
        this.storage = new storage_js_1.StorageClient(this.url, this.apikey, null);
        this.functions = new functions_js_1.FunctionsClient(this.url, this.apikey, null);
        this.graphql = new graphql_js_1.GraphQLClient(this.url, this.apikey, null);
        this.realtime = new realtime_js_1.RealtimeClient(this.url, this.apikey, null);
    }
    from(table) {
        const token = this.auth.session?.access_token || null;
        return new postgrest_js_1.QueryBuilder(this.url, this.apikey, token, table);
    }
    channel(topic) {
        const token = this.auth.session?.access_token || null;
        const realtime = new realtime_js_1.RealtimeClient(this.url, this.apikey, token);
        return realtime.channel(topic);
    }
}
exports.ChocoBaseClient = ChocoBaseClient;
function createClient(url, apikey, options) {
    return new ChocoBaseClient(url, apikey, options);
}
