use std::fs::File;
use std::io;
use std::io::Read;
use std::io::Seek;
use std::str;
use std::path::Path;
use byteorder::ReadBytesExt;

// BIOS Parameter Block,
// basic info about the volume
// not packed-able, some fields omitted
struct BootRecord {
    // size of a sector, in bytes (i.e. bytes per sector)
    // typically 0x200 (= 512)
    sector_size: u16,
    // size of a cluster, in sectors (i.e. sectors per cluster)
    // typically 2
    cluster_size: u8,
    // number of reserved sectors (incl. boot record)
    reserved_sectors: u16,
    // number of FATs (???), typically 2
    fat_count: u8,
    // number of root directory entries
    root_entries: u16,
    // total number of sectors (max. 64k, i.e. max total size 32M)
    // if 0, number is in large_sector_count
    sector_count: u16,
    // FAT size, in sectors (i.e. sectors/size)
    fat_size: u16,
    // hidden sectors: ???
    large_sector_count: u32,

    // extended boot record fields:
    _flags: u8,
    label: [u8;11],
}

impl BootRecord {
    fn parse(file: &mut File) -> io::Result<BootRecord> {
        use byteorder::LittleEndian;
        // skip boot jump and OEM identifier
        file.seek(io::SeekFrom::Start(11))?;

        let sector_size = file.read_u16::<LittleEndian>()?;
        let cluster_size = file.read_u8()?;
        let reserved_sectors = file.read_u16::<LittleEndian>()?;
        let fat_count = file.read_u8()?;
        let root_entries = file.read_u16::<LittleEndian>()?;
        let sector_count = file.read_u16::<LittleEndian>()?;
        // skip media parameter type
        let _ = file.read_u8()?;
        let fat_size = file.read_u16::<LittleEndian>()?;
        // skip drive geometry info
        let _ = file.read_u64::<LittleEndian>()?;
        let large_sector_count = file.read_u32::<LittleEndian>()?;
        // extended boot record
        // skip drive number
        let _ = file.read_u8()?;
        let _flags = file.read_u8()?;
        let signature = file.read_u8()?;
        assert!(signature == 0x28 || signature == 0x29);
        let mut label = [0u8 ; 11];
        let _ = file.read_u32::<LittleEndian>()?;
        file.read_exact(&mut label)?;

        Ok(BootRecord { sector_size, cluster_size, reserved_sectors,
                        fat_count, root_entries, sector_count,
                        fat_size, large_sector_count, _flags, label })
    }
}

pub struct FileSystem {
    file: std::fs::File,
    br: BootRecord,
}

impl FileSystem {
    pub fn new(path: &Path) -> io::Result<FileSystem> {
        let mut file = File::open(path)?;
        let br = BootRecord::parse(&mut file)?;

        Ok(FileSystem { file, br })
    }

    pub fn sectors_count(&self) -> u32 {
        if self.br.sector_count != 0 {
            self.br.sector_count as u32
        } else {
            self.br.large_sector_count
        }
    }

    pub fn volume_size(&self) -> u32 {
        self.sectors_count() * self.br.sector_size as u32
    }

    pub fn volume_name(&self) -> &str {
        str::from_utf8(&self.br.label).unwrap().trim_end()
    }

    fn fat_start_sector(&self) -> u32 {
        self.br.reserved_sectors as u32
    }

    fn root_start_sector(&self) -> u32 {
        self.fat_start_sector() + self.br.fat_count as u32 * self.br.fat_size as u32
    }

    pub fn root_directory(&self) -> Directory {
        Directory {
            inner: DirType::Root(self.root_start_sector(),
                                 self.br.root_entries)
        }
    }

    fn data_start_sector(&self) -> u32 {
        // max size of root dir, in bytes
        let root_size = self.br.root_entries << 5;
        self.root_start_sector() + (root_size / self.br.sector_size) as u32
    }

    fn cluster_start(&self, cluster: u16) -> u32 {
        self.data_start_sector() + (cluster-2) as u32 * self.br.cluster_size as u32
    }

    fn fat_lookup(&mut self, cluster: u16) -> io::Result<u16> {
        let seek = self.fat_start_sector() * self.br.sector_size as u32
            + ((cluster as u32) << 1);
        println!("{:x} {:x} {:x}", seek, self.fat_start_sector(), self.br.sector_size);
        self.file.seek(io::SeekFrom::Start(seek as u64))?;
        self.file.read_u16::<byteorder::LittleEndian>()
    }

    pub fn read_directory(&mut self, dir: Directory) -> io::Result<Vec<DirectoryEntry>> {
        let mut cluster = 0;
        let (start_sector, entry_count, is_root) = match dir.inner {
            DirType::Root(start, count) => (start as u32, count, true),
            DirType::Regular(start) => {
                cluster = start;
                let fat = self.fat_lookup(cluster)?;
                println!("read regular dir {:x} {:x}", fat, cluster);
                if fat < 2 { return Ok(Vec::new()) }
                (self.cluster_start(cluster),
                 (self.br.cluster_size as u16 * self.br.sector_size as u16) >> 5,
                 false)
            }
        };

        let seek = start_sector * self.br.sector_size as u32;
        self.file.seek(io::SeekFrom::Start(seek as u64))?;
        let mut entries = Vec::with_capacity(64);
        let mut count = 0;

        loop {
            use byteorder::LittleEndian;

            // end of current cluster?
            if count == entry_count {
                if is_root { break }
                else {
                    // next cluster?
                    if cluster > 0xffef { break }
                    cluster = self.fat_lookup(cluster)?;
                    if cluster < 2 { break }
                    // yes
                    let start_sector = self.cluster_start(cluster);
                    let seek = start_sector * self.br.sector_size as u32;
                    self.file.seek(io::SeekFrom::Start(seek as u64))?;
                    count = 0;
                }
            }
            
            let mut name = [0u8;8];
            let mut ext = [0u8;3];
            self.file.read_exact(&mut name)?;
            self.file.read_exact(&mut ext)?;

            if name[0] == 0 {
                break;
            }

            let flags = self.file.read_u8()?;
            // skip various fields
            self.file.seek(io::SeekFrom::Current(14))?;
            let first_cluster = self.file.read_u16::<LittleEndian>()?;
            let size = self.file.read_u32::<LittleEndian>()?;

            if flags != 0xf {
                entries.push(DirectoryEntry { name, ext, flags, first_cluster, size });
            }
            count += 1;
        }

        Ok(entries)
    }
}

pub struct File_ {
    first_cluster: u16,
    size: u32
}

pub enum DirType {
    // root dir: first sector, entry count
    Root(u32, u16),
    // regular dir: first cluster
    Regular(u16)
}

pub struct Directory {
    inner: DirType
}

pub struct DirectoryEntry {
    name: [u8 ; 8],
    ext: [u8 ; 3],
    flags: u8,
    first_cluster: u16,
    size: u32
}

pub enum EntryType {
    File(File_),
    Dir(Directory)
}

impl DirectoryEntry {
    pub fn name(&self) -> &str {
        str::from_utf8(&self.name).unwrap().trim_end()
    }

    pub fn extension(&self) -> &str {
        str::from_utf8(&self.ext).unwrap().trim_end()
    }

    pub fn entry_type(&self) -> EntryType {
        if self.flags & 0x10 != 0 {
            EntryType::Dir(Directory {
                inner: DirType::Regular(self.first_cluster)
            })
        } else {
            EntryType::File(File_ { first_cluster: self.first_cluster,
                                    size: self.size })
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
