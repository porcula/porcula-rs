#!/bin/env perl
# list or delete crates not used in _current_ project

use strict;
use autodie;
use JSON qw/decode_json/;
use File::Path qw/rmtree/;

my $dry_run = !grep { $_ eq 'delete' } @ARGV;

my $metadata = do {
  open my $f, "cargo metadata --quiet --offline --format-version=1 |";
  local $/;
  decode_json <$f>
};

my (%used, $src_dir);

for my $lib (@{ $metadata->{packages} }) {
  my $name = $lib->{name};
  my ($src) = 
    map { $_->{src_path} }
    grep { grep /lib|macro/, @{ $_->{kind} } }  #lib|cdylib|rlib|proc-macro
    @{ $lib->{targets} };
  my ($dir,$subdir) = $src =~ m!(^.+)[/\\]($name-[^/\\]+)[/\\].+! or next;
  $used{$subdir} = 1;
  $src_dir ||= $dir;
}

print "src_dir: $src_dir\n";
#for my $p (sort keys %used) { print "used: $p\n"; }

my $total = 0;
opendir my $d, $src_dir;
while (my $f = readdir($d)) {
  next if $f =~ /^\./;
  my $p = "$src_dir/$f";
  next unless -d $p;
  next if exists $used{$f};
  if ($dry_run) {
    print "probably unused $f\n";
  } else {
    print "delete $f\n";
    rmtree $p;
  }
  $total += 1;
}
closedir $d;
print "---\ntotal $total\n";