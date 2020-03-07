function esc(unsafe) {
    if (!(typeof (unsafe) === "string")) return unsafe;
    return unsafe.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;").replace(/'/g, "&#039;");
}

function is_ebook() {
    return !!window.navigator.userAgent.match(/QtEmbedded|QtWebEngine/);
}

if (!Math.trunc) { //polyfill for old eBooks
    Math.trunc = function (v) {
        v = +v;
        if (!isFinite(v)) return v;
        return (v - v % 1) || (v < 0 ? -0 : v === 0 ? v : 0);
    };
}

//certain olde browser have no localStorage and does not allow polyfill it
var storage = window.localStorage ? window.localStorage : {
    _data       : {},
    setItem     : function(id, val) { return this._data[id] = String(val); },
    getItem     : function(id) { return this._data.hasOwnProperty(id) ? this._data[id] : undefined; },
    removeItem  : function(id) { return delete this._data[id]; },
    clear       : function() { return this._data = {}; }
};

function size_pretty(s) {
    if (s < 1024) { return s + ""; }
    s = Math.trunc(s / 1024);
    if (s < 1024) { return s + "K"; }
    s = Math.trunc(s / 1024);
    if (s < 1024) { return s + "M"; }
    s = Math.trunc(s / 1024);
    return s + "G";
}

function sort_by_prop(obj,prop) {
    var keys = Object.keys(obj);
    return keys.sort(function(a,b) {
        var a = obj[a][prop];
        var b = obj[b][prop];
        if (a=="" || a==undefined) return +1; //to end
        if (b=="") return -1; //to end
        return a.localeCompare(b);
    });
}

$(document).ajaxStart(function () { $("#loading").show(); });
$(document).ajaxStop(function () { $("#loading").hide(); });
