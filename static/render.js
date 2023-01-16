var words = [];
var word_count = {}; // {word} -> [id_prefix,count]
var wc_idx = 0;
var wc_max = 0;
var prev_find = '';

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
if (storage.getItem("hide_words")) {
    $('.find_words').hide();
}  

function do_find(w) {
    if (prev_find!=w) {
        wc_idx = 0;
        prev_find = w;
    }
    var p = word_count[w];
    if (p==undefined) {
        var id_prefix = 'w-'+(wc_max++)+'-';
        // allow non-letter characters (punctuation) between words
        var re = new RegExp(w.replace(/\s+/g, '\\P{L}+'), 'giu');
        var idx = 0;
        $("div.body").each(function(){
            html = this.innerHTML.replace(re, function(m) { 
                return '<span class="word" id="'+id_prefix+(idx++)+'">'+m+'</span>';
            });
            if (idx>0) this.innerHTML = html;
        });
        word_count[w] = p = [ id_prefix, idx ];
    }
    if (wc_idx >= p[1]) {
        wc_idx = 0;
    } else {
        var id = '#' + p[0] + wc_idx;
        var e = $(id);
        if (e.length) {
            e.addClass('hl');
            e[0].scrollIntoView({ "block": "center" });
        }
        wc_idx += 1;
    }
}

function hide_words() {
    $('.find_words').hide();
    $('.word.hl').removeClass('hl');
    storage.setItem('hide_words','1');
}

function show_words() {
    $('.find_words').show();
    $('.word').addClass('hl');
    storage.setItem('hide_words','0');
}


$(".find_words .hide").click(hide_words);
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

//enumerate paragraphs
var para_num = 0;
var paragraphs = []; //index for closest_para()
$('p').each(function(){
    paragraphs.push(this);
    if (this.id) return;
    $(this).attr('id', '_p'+(++para_num));
});

//plain numeric reference: [N] -> <p>N</p>
//could be independent numeration for each chapter
var plain_refs = [];
$('sup').each(function(){ //<sup>[N]</sup>
    if (this.childElementCount>0) return;
    if ($(this).closest('a').length) return;
    var m = this.textContent.match(/^\[([0-9]+)\]$/);
    if (m && m.length==2) plain_refs.push([m[1],this]);
});
if (plain_refs.length==0 && $('a').length==0) { //no markup [N]
    var sup_num = 0;
    $('#content').find("*").addBack().contents().filter(function(){
        return this.nodeType==3 && this.textContent.match(/\[([0-9]+)\]/);
    })
    .each(function(){
        var html = esc(this.textContent).replace(/\[([0-9]+)\]/g, function(m,n){
            var id = "_sup"+(++sup_num);
            plain_refs.push([n,'#'+id]);
            return '<sup id="'+id+'">'+m+'</sup>';
        });
        $(this).replaceWith(html);
    });
}
if (plain_refs.length) {
    var targets = {}; //numeric paragraphs, last one is probably notes
    $('div, p, div>strong:only-child, p>strong:only-child').each(function(){ 
        if (this.childElementCount>0) return;
        var m = this.textContent.match(/^\s*([0-9]+)/);
        if (m && m.length==2) {
            var n = m[1];
            var node = (this.tagName=='P' || this.tagName=='DIV') ? this : this.parentNode;
            if (targets[n]) {
                targets[n].push(node);
            } else {
                targets[n] = [node];
            }
        }
    });
    for (var i=plain_refs.length-1; i>=0; i--) {
        var n = plain_refs[i][0];
        var t = targets[n];
        if (t != undefined && t.length>0) {
            var f = t.pop(); 
            var s = plain_refs[i][1];
            $(s).wrapInner('<a href="#'+f.id+'"/>');
        }
    }
}
plain_refs = undefined;

//notes and backlinks
$("a[href^='#']").each(function () {
    var a = $(this);
    var href = a.attr("href");
    var note = $(href);
    if (note.length) {
        var title = a.attr("title");
        if (!title) {
            title = note.contents().slice(0, 10).text();
            //whitespace or numeric content -> add next paragraph
            if (title.match(/^\s*[0-9]*\s*$/)) {
                title = note.nextAll().filter(function(){return this.textContent.match(/\S/)}).first().text();
            }
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
    titles.push([n, L, T, id]);
});
if (titles.length > 1) { //do not show empty TOC or one-line TOC
    var h = '';
    var p = -1;
    for (var i = 0; i < titles.length; i++) {
        var L = titles[i][1] - min_lvl;
        var T = titles[i][2];
        var id = titles[i][3];
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

//title closest to viewport' center
function closest_title() {
    var y = window.pageYOffset + window.innerHeight/2;
    var a = 0;
    var b = titles.length;
    while ((b-a)>1) {
        var i = a+Math.floor((b-a)/2);
        var yi = titles[i][0].offsetTop;
        if (y<yi) { 
            b = i;
        } else { 
            a = i;
        }
    }
    return titles[a][3];
}

//paragraph closest to viewport' center, binary search in paragraphs array
function closest_para() {
    var y = window.pageYOffset + window.innerHeight/2;
    var a = 0;
    var b = paragraphs.length;
    while ((b-a)>1) {
        var i = a+Math.floor((b-a)/2);
        var yi = paragraphs[i].offsetTop;
        if (y<yi) { 
            b = i;
        } else { 
            a = i;
        }
    }
    return paragraphs[a].id;
}

function show_toc() {
    var id = closest_title();
    $(".toc").show();
    $('.toc li').removeClass("current");
    //highlight closest title
    if (id) {
        var a$ = $('.toc a[href="#' + id + '"]');
        var a = a$.get(0);
        if (a) {
            a.scrollIntoView({ "block": "center" });
            a$.parent().addClass("current");
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

//read progress
var read_position = "";
var last_top = 0;
setInterval(function () {
    var top = window.pageYOffset;
    if (top != last_top) {
        last_top = top;
        save_state();            
    }
}, 10000);

//stored reading state: { book-id, last-read-date, position, bookmarks, current-bookmark, auto-bookmark }
var max_book_stored = 10;
var book_stored = 0;
var book_idx = null;
var min_idx = 0;
var min_d = '9999';
var book_id = window.location.pathname.replace('/porcula/book/','').replace('/render','');
var state = { id: book_id, p: "", m:[], c:0 };

for (var i=0; i<max_book_stored; i++) { //LRU cache
  var s =  storage.getItem("book"+i);
  if (!s || s=='') continue;
  book_stored = i;
  var b = JSON.parse(s);
  if (b.id==book_id) {
      book_idx = i;
      state = b;
  }
  if (b.d<min_d) { min_d=b.d; min_idx=i; }
}
if (book_idx==null) {
    if (book_stored>=max_book_stored-1) {
        book_idx = min_idx;
    }
    else {
        book_idx = book_stored+1;
    }
}
if (state.p) {
    var c = $('#'+state.p).get();
    if (c.length>0) {
        c[0].scrollIntoView({ "block": "center" });
    }
}
for (i in state.m) {
  $('#'+state.m[i]).addClass('bookmark bm'+i);
}

function save_state() {
    var id = closest_para();
    if (!id) return;
    if (id==read_position) return;
    read_position = id;
    window.history.replaceState(null, "", "#" + id);
    state.d = (new Date()).toISOString();
    state.p = id;
    storage.setItem('book'+book_idx, JSON.stringify(state));
}

function set_auto_bookmark() {
    var id = closest_para();
    if (!id) return;
    state.ab = id;
    var hash = '#'+id;
    window.history.pushState(null, '', hash);
}

function goto_auto_bookmark() {
    if (!state.ab) return;
    var e = $('#'+state.ab).get(0);
    if (!e) return;
    e.scrollIntoView({ "block": "center" });
}

function toggle_bookmark() {
    var id;
    var s = document.getSelection();
    if (s.type == 'Range') { //selected text
        var r = s.getRangeAt(0);
        var n = r.startContainer;
        while (n) {
            if (n.id) {
                id = n.id;
                break;
            }
            n = n.parentNode;
        }
    }
    if (!id) id = closest_para(); //paragraph in center of view
    if (!id) return;
    var i = state.m.indexOf(id);
    if (i<0) { //add
        for (var idx in state.m) { 
            if (state.m[idx]==undefined || state.m[idx]==null) {
                i = idx; //reuse undefined entry
                break;
            }
        }
        if (i<0) {
            i = state.m.push(id) - 1; //add new entry
        } else {
            state.m[i] = id;
        }
        state.c = i;
        $('#'+id).addClass('bookmark bm'+i);
    }
    else { //remove
        state.m[i] = null;
        var j = state.m.length;
        while (j>0 && state.m[j-1]==null) j--;
        state.m.length = j; //trim array
        state.c = (i>0) ? i-1 : 0;
        $('#'+id).removeClass('bookmark bm'+i);
    }
    save_state();  
}

function prev_bookmark() {
    if (state.m.length==0) return;
    var i = state.c;
    if (i<1) { 
        i = 0;
    }
    else if (i>(state.m.length-1)) {
        i = state.m.length-1;
    } else {   
        i--;
    }
    while (i>0 && !state.m[i]) {
        i--;
    }
    if (!state.m[i]) return;
    state.c = i;
    var e = $('#'+state.m[i]).get(0);
    if (!e) return;
    set_auto_bookmark();
    e.scrollIntoView({ "block": "center" });
    save_state();  
}

function next_bookmark() {
    if (state.m.length==0) return;
    var i = state.c;
    if (i<0) {
        i=0;
    } else if (i>=(state.m.length-1)) {
        i = state.m.length-1;
    } else {
        i++;
    }
    while (i<state.m.length && !state.m[i]) {
        i++;
    }
    if (!state.m[i]) return;
    state.c = i;
    var e = $('#'+state.m[i]).get(0);
    if (!e) return;
    set_auto_bookmark();
    e.scrollIntoView({ "block": "center" });
    save_state();  
}

function goto_bookmark(n) {
    if (n<0 || n>(state.m.length-1)) return;
    var e = $('#'+state.m[n]).get(0);
    if (!e) return;
    set_auto_bookmark();
    e.scrollIntoView({ "block": "center" });
    state.c = n;
    save_state();  
}

window.addEventListener('keydown', function (e) {
    if (!e) e = window.event;
    var code = e.code || e.keyCode;
    switch (code) {
        case 'KeyT': case 84:
            if ($(".toc:visible").length) {
                $(".toc").hide()
            }
            else {
                show_toc();
            }
            break;
        case 'Digit0': case 48:
            if (e.altKey) {
                goto_auto_bookmark();
            } else {
                $('.find_words').toggle();
                var hide = $('.find_words:visible').length==0 ? '1' : '';
                if (hide) { hide_words(); } else { show_words(); }
            }
            break;
        case 'Digit1': case 'Digit2': case 'Digit3': case 'Digit4': case 'Digit5': case 'Digit6': case 'Digit7': case 'Digit8': case 'Digit9':
        case 49: case 50: case 51: case 52: case 53: case 54: case 55: case 56: case 57: 
            e.preventDefault();
            var n = Number(e.key);
            if (e.altKey) {
                goto_bookmark(n-1);
            } else {
                if (n<=words.length) do_find(words[n-1]);
            }
            break;
        case 'Escape': case 27:
            $(".toc").hide(); 
            break;
        case 'KeyB': case 66:
            if (!e.ctrlKey && !e.altKey) toggle_bookmark();
            break;
        case 'KeyK': case 75:
            if (e.ctrlKey) toggle_bookmark();
            break;
        case 'KeyP': case 80:
            if (!e.ctrlKey && !e.altKey) prev_bookmark();
            break;
        case 'KeyN': case 78:
            if (!e.ctrlKey && !e.altKey) next_bookmark();
            break;
        case 'Home': case 'End': case 36: case 35:
            set_auto_bookmark();
            break;
    }
});
