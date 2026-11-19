require "digest"

url, archive = ARGV
abort "usage: ruby render.rb VERSIONED_SOURCE_URL SOURCE_ARCHIVE" unless ARGV.length == 2 &&
  url.match?(%r{\A(?:https://|file:///)[\w./-]+/v\d+\.\d+\.\d+\.tar\.gz\z})

replacements = { "@URL@" => url, "@SHA256@" => Digest::SHA256.file(archive).hexdigest }
puts File.read(File.join(__dir__, "foundry-opencode.rb.in")).gsub(/@(?:URL|SHA256)@/, replacements)
