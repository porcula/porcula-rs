var words = [];
var find_word = "";
var find_elem;
var find_start = 0;
var qs = (function (a) {
    if (a == "") return {};
    var b = {};
    for (var i = 0; i < a.length; ++i) {
        var p = a[i].split('=', 2);
        if (p.length == 1)
            b[p[0]] = "";
        else
            b[p[0]] = decodeURIComponent(p[1].replace(/\+/g, " "));
    }
    return b;
})(window.location.search.substring(1).split('&'));
if (qs["find"]) {
    words = qs["find"].split(',');
    if (words.length > 0) {
        var h = '<span class="hide">X</span>';
        for (var i in words) {
            h += '<span class="word" contenteditable>' + words[i] + '</span>';
        }
        $("body").append('<div class="find_words">' + h + '</div>');
    }
}

function do_find(w) {
    if (find_word != w || !find_elem) {
        find_elem = document.getRootNode();
        find_start = 0;
    }
    find_word = w;
    var r = new Range();
    r.setStart(find_elem, find_start);
    r.setEnd(find_elem, find_start);
    var s = document.getSelection();
    s.empty();
    s.addRange(r);
    if (window.find(find_word)) {
        s = document.getSelection();
        r = s.getRangeAt(0);
        find_elem = r.endContainer;
        find_start = r.endOffset;
        if (find_elem.parentNode.classList.contains("word")) {
            find_elem = null;
        }
    }
    else {
        find_elem = null;
    }
}

$(".find_words .hide").click(function () {
    $('.find_words').hide();
});
$(".find_words .word").click(function () {
    do_find($(this).text());
}).dblclick(function (e) {
    e.preventDefault();
    var r = new Range();
    r.setStart(this, 0);
    r.setEnd(this, $(this).text().length - 1);
    var s = document.getSelection();
    s.empty();
    s.addRange(r);
}).keydown(function (e) {
    if (e.keyCode == 13) {
        e.preventDefault();
        do_find($(this).text());
    }
});

//backlinks
$("a[href^='#']").each(function () {
    var a = $(this);
    var href = a.attr("href");
    var note = $(href);
    if (note.length > 0) {
        var title = a.attr("title");
        if (!title) {
            title = note.contents().slice(0, 10).text();
            title = title.replace(/\n+/g, "\n").replace(/^\s+/, "").replace(/^[0-9]+\n/, "");
            if (title.length > 600) {
                title = title.substring(0, 600) + "...";
            }
            a.attr("title", title);
        }
        if ($("a", note).length == 0) {
            var link = $("div.title:first", note);
            if (link.length == 0) {
                link = note;
            }
            var id = a.attr("id");
            if (!id) {
                id = "back-" + href.substring(1);
                a.attr("id", id);
            }
            link.wrapInner('<a class="backlink" href="#' + id + '"></a>');
        }
    }
});

//table of contents
var min_lvl = 100;
var titles = [];
var num = 0;
$(".title", $("div.body").first()).each(function (i, n) {
    var a = $(n);
    var T = a.text().trim();
    var L = a.parents().length;
    if (L < min_lvl) min_lvl = L;
    var id = a.attr("id");
    if (!id) {
        id = "title-" + num;
        a.attr("id", id);
        num++;
    }
    titles.push([L, T, id]);
});
if (titles.length > 1) { //do not show empty TOC or one-line TOC
    var h = '';
    var p = -1;
    for (var i = 0; i < titles.length; i++) {
        var L = titles[i][0] - min_lvl;
        var T = titles[i][1];
        var id = titles[i][2];
        for (var x = p; x > L; x--) { h += '</ul>'; }
        for (var x = p; x < L; x++) { h += '<ul>'; }
        h += '<li><a href="#' + id + '">' + T + '</a></li>';
        p = L;
    }
    for (var x = p; x > -1; x--) { h += '</ul>'; }
    $(h).appendTo($('.toc'));
}
else {
    $(".toc").remove();
}

//title closest to viewport
function closest_title_id() {
    var y = window.pageYOffset + window.innerHeight/2;
    var c = null;
    var id = null;
    $(".title").each(function(){
      if (c && this.offsetTop>y) {
        id = c.id;
        return false;
      }
      c = this;
      return true;
    });
    return id;
}

function show_toc() {
    var id = closest_title_id();
    $(".toc").show();
    $('.toc a').removeClass("current");
    //highlight closest title
    if (id) {
        var a$ = $('.toc a[href="#' + id + '"]');
        var a = a$.get(0);
        if (a) {
            a.scrollIntoView({ "block": "center" });
            a$.addClass("current");
            var r = new Range();
            r.setStart(a, 0);
            r.setEnd(a, 0);
            var s = document.getSelection();
            s.empty();
            s.addRange(r);
        }
    }
}
$(".show_toc").click(function (e) {
    show_toc();
    e.stopPropagation();
});
$(".toc .hide").click(function () {
    $(".toc").hide();
});
$(".toc").on("click", "a", function (e) {
    $(".toc").hide();
});
$(".body").click(function () {
    $(".toc").hide();
});

//save read progress (coarsely)
var last_fragment = "";
var last_top = 0;
setInterval(function () {
    var top = window.pageYOffset;
    if (top != last_top) {
        last_top = top;
        var id = closest_title_id();
        if (id && id != last_fragment) {
            history.replaceState(null, "", "#" + id);
            last_fragment = id;
        }
    }
}, 10000);

document.onkeydown = function (e) {
    if (!e) e = window.event;
    if (e.key == '0') {
        $('.find_words').toggle();
    }
    else if (e.key == 't') {
        if ($(".toc:visible").length) {
            $(".toc").hide()
        }
        else {
            show_toc();
        }
    }
    else if (e.key >= '1' && e.key <= '9' && e.key <= words.length.toString()) {
        e.preventDefault();
        do_find(words[Number(e.key) - 1]);
    }
}
