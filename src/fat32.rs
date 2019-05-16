use byteorder::ReadBytesExt;
use std::io::{Read, Seek};
use std::{fs, io, path, str};

// normally read from boot record,
// here assumed to be 512 bytes
const SECTOR_SIZE: u32 = 0x200;

pub struct FAT32 {
    // underlying file descriptor
    file: fs::File,

    // BIOS Parameter Block fields,
    // basic info about the volume:
    reserved_sectors: u32, // number of reserved sectors (incl boot record)
    sector_count: u32,     // total number of sectors on the FS
    fat_size: u32,         // size of a FAT, in sectors (i.e. sectors/size)
    root_dir: u32,         // first cluster of root directory
    label: [u8; 11],       // file system name (aka label)
}

impl FAT32 {
    pub fn new(path: &path::Path) -> io::Result<FAT32> {
        use byteorder::LittleEndian;

        // open the file descriptor and read the
        // information in the boot record sector
        let mut file = fs::File::open(path)?;

        // skip boot jump and OEM identifier
        file.seek(io::SeekFrom::Start(11))?;
        let sector_size = file.read_u16::<LittleEndian>()?;
        assert!(sector_size as u32 == SECTOR_SIZE);
        let cluster_size = file.read_u8()?;
        assert!(cluster_size == 1);
        let reserved_sectors = file.read_u16::<LittleEndian>()? as u32;
        let fat_count = file.read_u8()?;
        assert!(fat_count == 1);

        // extended FAT32 boot record
        file.seek(io::SeekFrom::Start(32))?;
        let sector_count = file.read_u32::<LittleEndian>()?;
        let fat_size = file.read_u32::<LittleEndian>()?;
        // skip flags and version
        let _flags = file.read_u16::<LittleEndian>()?;
        let _version = file.read_u16::<LittleEndian>()?;
        let root_dir = file.read_u32::<LittleEndian>()?;

        // label: 11 ascii bytes padded with spaces
        let mut label = [0u8; 11];
        file.seek(io::SeekFrom::Start(71))?;
        file.read_exact(&mut label)?;

        Ok(FAT32 {
            file,
            reserved_sectors,
            sector_count,
            fat_size,
            root_dir,
            label,
        })
    }

    pub fn sector_count(&self) -> u32 {
        // total number of sectors in the volume
        self.sector_count
    }

    pub fn volume_size(&self) -> u32 {
        // full size of the volume, in bytes
        self.sector_count() * SECTOR_SIZE
    }

    pub fn volume_name(&self) -> &str {
        // remove padding spaces in volume name
        str::from_utf8(&self.label).unwrap().trim_end()
    }

    fn fat_start_sector(&self) -> u32 {
        // FAT starts after reserved sectors
        self.reserved_sectors
    }

    fn data_start_sector(&self) -> u32 {
        // data (i.e. clusters) start after FAT
        self.fat_start_sector() + self.fat_size
    }

    fn cluster_start(&self, cluster: u32) -> u32 {
        // clusters start at the first sector after
        // the reserved sectors and the FAT.
        // clusters 0 and 1 have entries in the FAT
        // but do not actually exist on disk (hence -2)
        assert!(cluster >= 2);
        self.data_start_sector() + (cluster - 2)
    }

    fn fat_lookup(&mut self, cluster: u32) -> io::Result<u32> {
        // read the FAT entry describing a given cluster
        // seek offset: beginning of FAT (in bytes) + 4 bytes / entry
        let seek = self.fat_start_sector() * SECTOR_SIZE + (cluster << 2);
        self.file.seek(io::SeekFrom::Start(seek as u64))?;
        self.file.read_u32::<byteorder::LittleEndian>()
    }

    pub fn root_directory(&self) -> Directory {
        // root directory is in the FAT, at a cluster
        // given in the boot record
        Directory {
            cluster: self.root_dir,
        }
    }

    pub fn read_directory(&mut self, dir: Directory) -> io::Result<Vec<DirectoryEntry>> {
        let mut cluster = dir.cluster;
        // entries per cluster: sector size / 32
        let count = SECTOR_SIZE >> 5;
        // vector initial capacity: 1 sector
        // (will automatically grow if overflow)
        let mut entries = Vec::with_capacity(count as usize);

        'outer: while cluster < 0xfffff0 {
            use byteorder::LittleEndian;

            // seek to beginning of cluster
            let seek = self.cluster_start(cluster) * SECTOR_SIZE;
            self.file.seek(io::SeekFrom::Start(seek as u64))?;

            // read directory entries until we reach maximum number
            // of entries/sector OR reach a termination marker
            for _ in 0..count {
                let mut name = [0u8; 11];
                self.file.read_exact(&mut name)?;

                if name[0] == 0 {
                    // end marker
                    break 'outer;
                }

                let flags = self.file.read_u8()?;
                // skip various fields
                // TODO: hi bytes of cluster num
                self.file.seek(io::SeekFrom::Current(14))?;
                let cluster = self.file.read_u16::<LittleEndian>()? as u32;
                let size = self.file.read_u32::<LittleEndian>()?;

                // flag 0xf = special entry for long filenames
                // not supported atm.
                if flags != 0xf {
                    entries.push(DirectoryEntry {
                        name,
                        flags,
                        cluster,
                        size,
                    });
                }
            }

            // end of sector, read next cluster
            cluster = self.fat_lookup(cluster)?;
        }

        Ok(entries)
    }
}

// describes one entry in
// a directory listing
pub struct DirectoryEntry {
    name: [u8; 11],
    flags: u8,
    cluster: u32,
    size: u32,
}

pub enum EntryType {
    File(File),
    Dir(Directory),
}

pub struct File {
    cluster: u32,
    size: u32,
}

pub struct Directory {
    cluster: u32,
}

impl DirectoryEntry {
    pub fn name(&self) -> &str {
        // removes the padding spaces around the name
        str::from_utf8(&self.name[..8]).unwrap().trim_end()
    }

    pub fn extension(&self) -> &str {
        // removes the padding spaces around the extension
        str::from_utf8(&self.name[8..]).unwrap().trim_end()
    }

    pub fn full_name(&self) -> String {
        // returns the full name of the file : NAME.EXT
        // uses a buffered String to concatenate name and ext
        let mut name = String::with_capacity(12);
        name.push_str(self.name());
        let ext = self.extension();
        if ext != "" {
            name.push('.');
            name.push_str(ext);
        }
        return name;
    }

    pub fn entry_type(&self) -> EntryType {
        // 0x10 = 00010000
        // 5th bit of flags = directory or file
        if self.flags & 0x10 != 0 {
            EntryType::Dir(Directory {
                cluster: self.cluster,
            })
        } else {
            EntryType::File(File {
                cluster: self.cluster,
                size: self.size,
            })
        }
    }
}
