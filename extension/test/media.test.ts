import { describe, expect, it } from 'vitest';
import { type MediaItem, addMedia, classifyMedia, disguiseCandidate, elementMedia, elementSources, humanSize, isIgnoredMediaUrl, linkedinCommentaryPath, linkedinPostPath, manifestKind, mediaDetail, mediaKey, mediaLabel, postUrl, qualityFromUrl, totalSize, visibleMedia } from '../src/shared/media';

const LI = 'https://dms.licdn.com/playlist/vid/v2/D5610AQFKo9M0zqY2_g';

const item = (over: Partial<MediaItem>): MediaItem => ({
  key: over.url ?? 'k',
  url: 'https://cdn.x.com/v.mp4',
  kind: 'video',
  streamKind: 'direct',
  name: 'v.mp4',
  ts: 0,
  ...over,
});

describe('classifyMedia', () => {
  it('detects manifests by type and by extension', () => {
    expect(classifyMedia({ url: 'https://x/master.m3u8', contentType: 'application/vnd.apple.mpegurl' })?.kind).toBe('hls');
    expect(classifyMedia({ url: 'https://x/a', contentType: 'application/x-mpegURL' })?.kind).toBe('hls');
    expect(classifyMedia({ url: 'https://x/live.m3u8?t=1', contentType: 'text/plain' })?.kind).toBe('hls');
    expect(classifyMedia({ url: 'https://x/manifest.mpd', contentType: 'application/dash+xml' })?.streamKind).toBe('dash');
    expect(classifyMedia({ url: 'https://x/manifest.mpd', contentType: '' })?.kind).toBe('dash');
  });
  it('detects progressive media and segments', () => {
    expect(classifyMedia({ url: 'https://x/a.mp4', contentType: 'video/mp4' })?.kind).toBe('video');
    expect(classifyMedia({ url: 'https://x/a', contentType: 'audio/mpeg' })?.kind).toBe('audio');
    expect(classifyMedia({ url: 'https://x/a.webm', contentType: 'application/octet-stream' })?.kind).toBe('video');
    expect(classifyMedia({ url: 'https://x/seg-12.ts', contentType: 'video/mp2t' })?.kind).toBe('segment');
    expect(classifyMedia({ url: 'https://x/chunk-3.m4s', contentType: 'video/iso.segment' })?.kind).toBe('segment');
  });
  it('ignores non-media', () => {
    expect(classifyMedia({ url: 'https://x/page.html', contentType: 'text/html' })).toBeNull();
    expect(classifyMedia({ url: 'https://x/a.mp4', contentType: 'text/html' })).toBeNull();
    expect(classifyMedia({ url: 'https://x/a.png', contentType: 'image/png' })).toBeNull();
    expect(classifyMedia({ url: 'https://x/a.js', contentType: 'application/javascript' })).toBeNull();
    expect(isIgnoredMediaUrl('https://rr1---sn-abc.googlevideo.com/videoplayback?x=1')).toBe(true);
    expect(isIgnoredMediaUrl('blob:https://x/1')).toBe(true);
    expect(isIgnoredMediaUrl('https://cdn.x/v.mp4')).toBe(false);
  });
});

describe('disguiseCandidate / manifestKind (click-time manifest check)', () => {
  it('flags only a player XHR/media fetch disguised as an image', () => {
    expect(disguiseCandidate({ url: 'https://x/playlist.png', contentType: 'image/png', resourceType: 'xmlhttprequest' })).toBe(true);
    expect(disguiseCandidate({ url: 'https://x/playlist.png', contentType: 'image/png', resourceType: 'media' })).toBe(true);
    expect(disguiseCandidate({ url: 'https://x/a', contentType: 'image/jpeg', resourceType: 'xmlhttprequest' })).toBe(true);
    expect(disguiseCandidate({ url: 'https://x/a.webp', contentType: '', resourceType: 'media' })).toBe(true);
  });
  it('ignores a real <img>/frame load and anything not image-shaped', () => {
    expect(disguiseCandidate({ url: 'https://x/playlist.png', contentType: 'image/png', resourceType: 'object' })).toBe(false);
    expect(disguiseCandidate({ url: 'https://x/playlist.png', contentType: 'image/png', resourceType: 'main_frame' })).toBe(false);
    expect(disguiseCandidate({ url: 'https://x/a.json', contentType: 'application/json', resourceType: 'xmlhttprequest' })).toBe(false);
    expect(disguiseCandidate({ url: 'https://x/a.txt', contentType: 'text/plain', resourceType: 'xmlhttprequest' })).toBe(false);
  });
  it('reads the manifest magic bytes from a body prefix', () => {
    expect(manifestKind('#EXTM3U\n#EXT-X-VERSION:3')).toBe('hls');
    expect(manifestKind('  \n#EXTM3U')).toBe('hls');
    expect(manifestKind('<?xml version="1.0"?>\n<MPD xmlns="urn:mpeg:dash:schema:mpd:2011">')).toBe('dash');
    expect(manifestKind('not a manifest')).toBeNull();
  });
});

describe('helpers', () => {
  it('normalizes media keys', () => {
    expect(mediaKey('https://x/v.mp4?range=0-100&id=7#t=5')).toBe('https://x/v.mp4?id=7');
    expect(mediaKey('https://x/v.mp4?id=7&bytes=1-2')).toBe(mediaKey('https://x/v.mp4?id=7&bytes=3-4'));
  });
  it('computes total size from 206 and 200', () => {
    expect(totalSize(206, '1024', 'bytes 0-1023/987654')).toBe(987654);
    expect(totalSize(200, '5000')).toBe(5000);
    expect(totalSize(200, undefined)).toBeUndefined();
    expect(totalSize(206, '10', 'bytes 0-9/*')).toBe(10);
  });
  it('guesses quality', () => {
    expect(qualityFromUrl('https://x/video_1080p.mp4')).toBe('1080p');
    expect(qualityFromUrl('https://x/hls/720/index.m3u8')).toBe('720p');
    expect(qualityFromUrl('https://x/movie-4k.mkv')).toBe('2160p');
    expect(qualityFromUrl('https://x/id/123456/v.mp4')).toBeUndefined();
    // LinkedIn: dms.licdn.com/playlist/vid/v2/<asset>/mp4-640p-30fp-crf28/…/<timestamp>
    expect(qualityFromUrl(`${LI}/mp4-640p-30fp-crf28/mp4-640p-30fp-crf28/0/1704988851446?e=2147483647&v=beta&t=x`)).toBe('640p');
    expect(qualityFromUrl('https://x/v/1704988851446/clip?id=96000')).toBeUndefined();
  });
  it('shows the file name, or the server for CDN URLs that end in IDs', () => {
    expect(mediaDetail(item({ url: 'https://cdn.x.com/media/clip.mp4', name: 'clip.mp4' }))).toBe('clip.mp4');
    expect(mediaDetail(item({ url: `${LI}/mp4-640p-30fp-crf28/0/1704988851446?e=1`, name: '1704988851446' }))).toBe('dms.licdn.com');
    expect(mediaDetail(item({ url: 'https://scontent-prg1-1.cdninstagram.com/o1/v/t16/f2/m86/AQO1ALoA6dTGFDP9ozZI3MuLOqC00q?efg=x', name: 'AQO1ALoA6dTGFDP9ozZI3MuLOqC00q' }))).toBe('scontent-prg1-1.cdninstagram.com');
  });
  it('labels streams by quality and container, also from the MIME type', () => {
    expect(mediaLabel(item({ url: `${LI}/mp4-640p-30fp-crf28/0/1704988851446`, mime: 'video/mp4', quality: '640p', size: 5316468 }))).toBe('640p · MP4 · 5.1 MB');
    expect(mediaLabel(item({ url: 'https://x/clip.webm', size: 300 * 1024 }))).toBe('Video · WEBM · 300 KB');
    expect(mediaLabel(item({ url: 'https://x/master.m3u8', kind: 'hls', streamKind: 'hls' }))).toBe('Stream · HLS');
  });
  it('formats sizes', () => {
    expect(humanSize(0)).toBe('');
    expect(humanSize(512)).toBe('512 B');
    expect(humanSize(1536)).toBe('1.5 KB');
    expect(humanSize(145 * 1024 * 1024)).toBe('145 MB');
  });
});

describe('addMedia', () => {
  it('dedupes, grows size, prunes segments once a manifest is known', () => {
    let list: MediaItem[] = [];
    list = addMedia(list, item({ url: 's1.ts', key: 's1', kind: 'segment', size: 500_000 })).list;
    expect(list).toHaveLength(1);
    list = addMedia(list, item({ url: 'v.mp4', key: 'v', size: 100_000 })).list;
    const again = addMedia(list, item({ url: 'v.mp4', key: 'v', size: 90_000 }));
    expect(again.changed).toBe(false);
    const bigger = addMedia(list, item({ url: 'v.mp4', key: 'v', size: 900_000 }));
    expect(bigger.changed).toBe(true);
    list = bigger.list;
    expect(list.find((m) => m.key === 'v')?.size).toBe(900_000);
    list = addMedia(list, item({ url: 'm.m3u8', key: 'm', kind: 'hls', streamKind: 'hls' })).list;
    expect(list.some((m) => m.kind === 'segment')).toBe(false);
    expect(addMedia(list, item({ url: 's2.ts', key: 's2', kind: 'segment', size: 500_000 })).changed).toBe(false);
    expect(visibleMedia(list)).toHaveLength(2);
  });
  it('drops tiny progressive files and caps the list', () => {
    expect(addMedia([], item({ size: 1000 })).changed).toBe(false);
    let list: MediaItem[] = [];
    for (let i = 0; i < 120; i++) list = addMedia(list, item({ key: `k${i}`, url: `u${i}` }), 100).list;
    expect(list).toHaveLength(100);
    expect(list[0]!.key).toBe('k20');
  });
});

describe('element sources (the hovered <video>)', () => {
  const page = 'https://www.linkedin.com/feed/';
  const dataSources = JSON.stringify([
    { src: `${LI}/mp4-640p-30fp-crf28/0/1?e=1&v=beta&t=a`, type: 'video/mp4', 'data-bitrate': 125498 },
    { src: `${LI}/mp4-360p-30fp-crf28/0/2?e=1&v=beta&t=b`, type: 'video/mp4' },
    { src: `${LI}/mp4-720p-30fp-crf28/0/3?e=1&v=beta&t=c`, type: 'video/mp4' },
  ]);

  it('collects what the element plays, its src, <source>s and data-sources, without duplicates', () => {
    const got = elementSources(
      {
        currentSrc: `${LI}/mp4-640p-30fp-crf28/0/1?e=1&v=beta&t=a`,
        src: `${LI}/mp4-640p-30fp-crf28/0/1?e=1&v=beta&t=a`,
        sources: [{ url: '/media/fallback.webm', type: 'video/webm' }],
        dataSources,
      },
      page,
    );
    expect(got).toEqual([
      { url: `${LI}/mp4-640p-30fp-crf28/0/1?e=1&v=beta&t=a`, type: 'video/mp4' },
      { url: 'https://www.linkedin.com/media/fallback.webm', type: 'video/webm' },
      { url: `${LI}/mp4-360p-30fp-crf28/0/2?e=1&v=beta&t=b`, type: 'video/mp4' },
      { url: `${LI}/mp4-720p-30fp-crf28/0/3?e=1&v=beta&t=c`, type: 'video/mp4' },
    ]);
  });

  it('skips blob:/data: players and junk data-sources', () => {
    expect(elementSources({ currentSrc: 'blob:https://www.linkedin.com/5f1c', src: '', dataSources: 'not json' }, page)).toEqual([]);
    expect(elementSources({ currentSrc: 'data:video/mp4;base64,AAAA', dataSources: '[1, {"src": 5}, {"src": "javascript:alert(1)"}]' }, page)).toEqual([]);
  });

  it('puts the element first: sniffed details kept, other qualities added, best first', () => {
    const playing = item({
      key: mediaKey(`${LI}/mp4-640p-30fp-crf28/0/1?e=1&v=beta&t=a`),
      url: `${LI}/mp4-640p-30fp-crf28/0/1?e=1&v=beta&t=a`,
      quality: '640p',
      size: 5316468,
      headers: { Referer: 'https://www.linkedin.com/' },
    });
    const ad = item({ key: 'https://dms.licdn.com/ad.mp4', url: 'https://dms.licdn.com/ad.mp4', size: 40_000_000 });
    const own = elementMedia(elementSources({ currentSrc: playing.url, dataSources }, page), [ad, playing], page);
    expect(own.map((m) => m.quality)).toEqual(['720p', '640p', '360p']);
    expect(own[1]).toBe(playing);
    expect(own[0]).toMatchObject({ kind: 'video', streamKind: 'direct', mime: 'video/mp4', headers: { Referer: 'https://www.linkedin.com/' } });
    expect(own.some((m) => m.url === ad.url)).toBe(false);
  });

  it('keeps the element order when qualities are unknown, and falls back to the page as Referer', () => {
    const own = elementMedia([{ url: 'https://cdn.x.com/a/clip' }, { url: 'https://cdn.x.com/b/clip.webm' }, { url: 'https://cdn.x.com/master.m3u8' }], [], 'https://x.com/post/1');
    expect(own.map((m) => [m.url.split('/').slice(-2).join('/'), m.kind])).toEqual([
      ['a/clip', 'video'],
      ['b/clip.webm', 'video'],
      ['cdn.x.com/master.m3u8', 'hls'],
    ]);
    expect(own[0]!.headers).toEqual({ Referer: 'https://x.com/post/1' });
  });
});

describe('postUrl (the hovered post on a feed)', () => {
  it('turns post links into canonical single-post URLs', () => {
    expect(postUrl('/nasa/status/1846951300071829923/photo/1', 'https://x.com/home')).toBe('https://x.com/nasa/status/1846951300071829923');
    expect(postUrl('https://twitter.com/nasa/status/123', 'https://x.com/nasa')).toBe('https://twitter.com/nasa/status/123');
    expect(postUrl('/i/web/status/123', 'https://x.com/home')).toBe('https://x.com/i/web/status/123');
    expect(postUrl('/r/videos/comments/1g4abcd/funny_cat/', 'https://www.reddit.com/r/videos/')).toBe('https://www.reddit.com/r/videos/comments/1g4abcd');
    expect(postUrl('/p/DBx1yZ2abc/', 'https://www.instagram.com/')).toBe('https://www.instagram.com/p/DBx1yZ2abc');
    expect(postUrl('/reel/DBx1yZ2abc/?utm_source=ig', 'https://www.instagram.com/')).toBe('https://www.instagram.com/reel/DBx1yZ2abc');
    expect(postUrl('/@scout2015/video/6718335390845095173', 'https://www.tiktok.com/foryou')).toBe('https://www.tiktok.com/@scout2015/video/6718335390845095173');
    expect(postUrl('https://www.facebook.com/NASA/videos/1234567890/', 'https://www.facebook.com/')).toBe('https://www.facebook.com/NASA/videos/1234567890');
    expect(postUrl('https://www.facebook.com/watch/?v=1234567890&ref=x', 'https://www.facebook.com/')).toBe('https://www.facebook.com/watch/?v=1234567890');
    expect(postUrl('https://www.youtube.com/watch?v=aqz-KE-bpKQ&t=3', 'https://www.youtube.com/@BlenderStudio')).toBe('https://www.youtube.com/watch?v=aqz-KE-bpKQ');
    expect(postUrl('/videos/2271453223', 'https://www.twitch.tv/directory')).toBe('https://www.twitch.tv/videos/2271453223');
    expect(postUrl('/76979871', 'https://vimeo.com/channels/staffpicks')).toBe('https://vimeo.com/76979871');
    expect(postUrl('/feed/update/urn:li:activity:7151241570371948544/', 'https://www.linkedin.com/feed/')).toBe('https://www.linkedin.com/feed/update/urn:li:activity:7151241570371948544');
    expect(postUrl('/reel/DBx1yZ2abc/', 'https://www.instagram.com/explore/')).toBe('https://www.instagram.com/reel/DBx1yZ2abc');
  });

  it('ignores profiles, tags, listings and other sites', () => {
    const page = 'https://x.com/home';
    for (const href of ['/nasa', '/hashtag/space', '/nasa/photo', '/search?q=x', 'https://example.com/a/status/1', 'javascript:void(0)', '', null]) {
      expect(postUrl(href, page)).toBeUndefined();
    }
    expect(postUrl('/r/videos/', 'https://www.reddit.com/')).toBeUndefined();
    expect(postUrl('/user/someone/', 'https://www.reddit.com/')).toBeUndefined();
    expect(postUrl('/NASA/videos', 'https://www.facebook.com/')).toBeUndefined();
    expect(postUrl('/channels/staffpicks', 'https://vimeo.com/')).toBeUndefined();
  });

  it('recognises pages that already are a single post (their URL is used as is)', () => {
    for (const page of ['https://x.com/nasa/status/123', 'https://www.youtube.com/watch?v=aqz-KE-bpKQ', 'https://www.reddit.com/r/videos/comments/abc/t/', 'https://www.youtube.com/shorts/abc123']) {
      expect(postUrl(page, page)).toBeDefined();
    }
    for (const page of ['https://www.linkedin.com/posts/the-mathworks_2_what-is-mathworks-cloud-center-activity-7151241570371948544-4Gu7', 'https://www.linkedin.com/feed/update/urn:li:activity:7151241570371948544/', 'https://www.instagram.com/reel/DBx1yZ2abc/']) {
      expect(postUrl(page, page)).toBeDefined();
    }
    for (const page of ['https://x.com/home', 'https://www.reddit.com/r/videos/', 'https://www.instagram.com/', 'https://www.instagram.com/explore/', 'https://www.linkedin.com/feed/', 'https://www.twitch.tv/somechannel', 'https://www.youtube.com/']) {
      expect(postUrl(page, page)).toBeUndefined();
    }
  });
});


describe('linkedinPostPath (LinkedIn feed posts)', () => {
  it('finds the post id, also inside composite ids, nearest container first', () => {
    expect(linkedinPostPath(['ember123', 'urn:li:activity:7151241570371948544'])).toBe('/feed/update/urn:li:activity:7151241570371948544/');
    expect(linkedinPostPath(['urn:li:fsd_update:(urn:li:activity:7151241570371948544,MAIN_FEED,EMPTY,DEFAULT,false)'])).toBe('/feed/update/urn:li:activity:7151241570371948544/');
    expect(linkedinPostPath(['urn:li:ugcPost:6850898786781339649'])).toBe('/feed/update/urn:li:ugcPost:6850898786781339649/');
    // A reshare: the original post (inner container) comes first.
    expect(linkedinPostPath(['urn:li:activity:111111111', 'urn:li:activity:222222222'])).toBe('/feed/update/urn:li:activity:111111111/');
  });
  it('ignores other ids', () => {
    expect(linkedinPostPath(['urn:li:member:12345678', 'urn:li:share:7151241570371948544', '', null, undefined])).toBeUndefined();
  });
  it('the paths count as single posts', () => {
    expect(postUrl('/feed/update/urn:li:ugcPost:6850898786781339649/', 'https://www.linkedin.com/feed/')).toBe('https://www.linkedin.com/feed/update/urn:li:ugcPost:6850898786781339649');
  });
});

describe('linkedinCommentaryPath (LinkedIn newer UI: the post text block)', () => {
  const key = (variant: string) =>
    `translatable-commentary-FeTranslationUrn(contentUrnCommentUrn=null, contentUrnGroupPostUrn=null, ${variant}, detectedLocale=, targetLocale=cs)`;
  it('reads a ugcPost or share id from the variant that is set', () => {
    expect(linkedinCommentaryPath(key('contentUrnShareUrn=null, contentUrnUgcPostUrn=ContentUrnUgcPostUrn(userGeneratedContentPostUrn=UserGeneratedContentPostUrn(userGeneratedContentId=7516114814234959872))'))).toBe(
      '/feed/update/urn:li:ugcPost:7516114814234959872/',
    );
    expect(linkedinCommentaryPath(key('contentUrnShareUrn=ContentUrnShareUrn(shareUrn=ShareUrn(shareId=7400000000000000123)), contentUrnUgcPostUrn=null'))).toBe(
      '/feed/update/urn:li:share:7400000000000000123/',
    );
  });
  it('skips other kinds, null variants and short numbers', () => {
    expect(
      linkedinCommentaryPath('translatable-commentary-FeTranslationUrn(contentUrnCommentUrn=null, contentUrnGroupPostUrn=ContentUrnGroupPostUrn(groupPostId=7400000000000000999), contentUrnShareUrn=null, contentUrnUgcPostUrn=null)'),
    ).toBeUndefined();
    expect(linkedinCommentaryPath(key('contentUrnShareUrn=null, contentUrnUgcPostUrn=null'))).toBeUndefined();
    expect(linkedinCommentaryPath(key('contentUrnShareUrn=null, contentUrnUgcPostUrn=ContentUrnUgcPostUrn(id=12345)'))).toBeUndefined();
    expect(linkedinCommentaryPath('update-card-focus3f9aFeedType_FLAGSHIP_SEARCH')).toBeUndefined();
    expect(linkedinCommentaryPath(null)).toBeUndefined();
  });
  it('the paths count as single posts', () => {
    expect(postUrl('/feed/update/urn:li:share:7400000000000000123/', 'https://www.linkedin.com/search/results/content/')).toBe('https://www.linkedin.com/feed/update/urn:li:share:7400000000000000123');
  });
});
