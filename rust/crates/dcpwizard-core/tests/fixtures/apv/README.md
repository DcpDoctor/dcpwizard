# APV fixture

`testsrc_6f_2048x1080.apv`: six frames of ffmpeg's `testsrc` in a raw APV
container, yuv422p10le, made with

    ffmpeg -f lavfi -i "testsrc=duration=0.25:size=2048x1080:rate=24" \
      -c:v apv -pix_fmt yuv422p10le testsrc_6f_2048x1080.apv

It is checked in because the `apv` encoder needs ffmpeg built with libopenapv,
which the conda-forge build the macOS runner uses does not have. Decoding APV
is native to ffmpeg 8, so every runner can read it.
