//! ingest-package: a real PCM MXF present in a folder but omitted from the
//! ASSETMAP/PKL (the exported OV/VF case) is re-declared under its embedded
//! asset UUID after repackaging.

use dcpwizard_core::ingest_package::ingest_package;
use dcpwizard_core::mxf_wrap::{MxfType, MxfWrapConfig, wrap_mxf_result};
use std::io::Write;
use std::path::Path;

/// Minimal 6-channel 24-bit 48 kHz WAV with silence, enough for asdcplib.
fn write_wav(path: &Path) {
    let (channels, bits, sample_rate): (u16, u16, u32) = (6, 24, 48_000);
    let block_align = (bits / 8) * channels;
    let data_len = block_align as u32 * 2000;
    let mut w = Vec::new();
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVE");
    w.extend_from_slice(b"fmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&channels.to_le_bytes());
    w.extend_from_slice(&sample_rate.to_le_bytes());
    w.extend_from_slice(&(sample_rate * block_align as u32).to_le_bytes());
    w.extend_from_slice(&block_align.to_le_bytes());
    w.extend_from_slice(&bits.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    w.extend_from_slice(&vec![0u8; data_len as usize]);
    std::fs::File::create(path).unwrap().write_all(&w).unwrap();
}

#[test]
fn repackage_declares_present_mxf_omitted_from_assetmap() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    // wrap a real PCM MXF; its embedded asset UUID is what the ASSETMAP must carry
    let wav = root.join("in.wav");
    write_wav(&wav);
    let sound_mxf = root.join("sound.mxf");
    let track = wrap_mxf_result(&MxfWrapConfig {
        input_path: wav.clone(),
        output_mxf: sound_mxf.clone(),
        mxf_type: MxfType::PcmAudio,
        frame_rate: 24,
        ..Default::default()
    })
    .expect("wrap sound MXF");
    let snd_id = track.uuid.clone();
    std::fs::remove_file(&wav).unwrap();

    // a CPL referencing the sound MXF by its real id
    let cpl_id = "33333333-3333-3333-3333-333333333333";
    let cpl = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL">
  <Id>urn:uuid:{cpl_id}</Id>
  <ContentTitleText>Ingest Test</ContentTitleText>
  <ReelList>
    <Reel>
      <Id>urn:uuid:55555555-5555-5555-5555-555555555555</Id>
      <AssetList>
        <MainSound>
          <Id>urn:uuid:{snd_id}</Id>
          <EditRate>24 1</EditRate>
          <IntrinsicDuration>2</IntrinsicDuration>
          <Duration>2</Duration>
        </MainSound>
      </AssetList>
    </Reel>
  </ReelList>
</CompositionPlaylist>
"#
    );
    let cpl_path = root.join(format!("CPL_{cpl_id}.xml"));
    std::fs::write(&cpl_path, cpl).unwrap();

    // an incomplete ASSETMAP/PKL: the sound MXF is NOT listed (the bug)
    let pkl_id = "44444444-4444-4444-4444-444444444444";
    std::fs::write(
        root.join(format!("PKL_{pkl_id}.xml")),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<PackingList xmlns="http://www.smpte-ra.org/schemas/429-8/2007/PKL">
  <Id>urn:uuid:{pkl_id}</Id>
  <AssetList>
    <Asset>
      <Id>urn:uuid:{cpl_id}</Id>
      <Hash>reusedcplhashAAAAAAAAAAAAAAA=</Hash>
      <Size>10</Size>
      <Type>text/xml</Type>
    </Asset>
  </AssetList>
</PackingList>
"#
        ),
    )
    .unwrap();
    std::fs::write(
        root.join("ASSETMAP.xml"),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:66666666-6666-6666-6666-666666666666</Id>
  <AssetList>
    <Asset>
      <Id>urn:uuid:{pkl_id}</Id>
      <PackingList>true</PackingList>
      <ChunkList><Chunk><Path>PKL_{pkl_id}.xml</Path></Chunk></ChunkList>
    </Asset>
    <Asset>
      <Id>urn:uuid:{cpl_id}</Id>
      <ChunkList><Chunk><Path>CPL_{cpl_id}.xml</Path></Chunk></ChunkList>
    </Asset>
  </AssetList>
</AssetMap>
"#
        ),
    )
    .unwrap();

    assert_eq!(ingest_package(root, None), 0);

    // the regenerated ASSETMAP now maps the present MXF to its real asset id
    let am = std::fs::read_to_string(root.join("ASSETMAP.xml")).unwrap();
    assert!(
        am.contains(&snd_id),
        "ASSETMAP must declare the sound MXF id"
    );
    assert!(
        am.contains("<Path>sound.mxf</Path>"),
        "ASSETMAP maps the MXF file"
    );
    assert!(am.contains(&format!("<Path>CPL_{cpl_id}.xml</Path>")));

    // a single fresh PKL declares the MXF (application/mxf) and reuses the CPL hash
    let pkl_name = std::fs::read_dir(root)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| n.starts_with("PKL_") && n.ends_with(".xml"))
        .expect("a PKL was written");
    let pkl = std::fs::read_to_string(root.join(&pkl_name)).unwrap();
    assert!(pkl.contains(&snd_id), "PKL must declare the sound MXF");
    assert!(pkl.contains("<Type>application/mxf</Type>"));
    assert!(
        pkl.contains("reusedcplhashAAAAAAAAAAAAAAA="),
        "existing CPL hash reused, not recomputed"
    );
    // the old PKL was replaced, not left behind
    assert!(!root.join(format!("PKL_{pkl_id}.xml")).exists());
    assert!(
        root.join("VOLINDEX.xml").exists(),
        "VOLINDEX written for SMPTE"
    );
}
