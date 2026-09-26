//! LICENSE bodies for `soroban-forge new --license <id>`.
//!
//! Each id maps to a well-known license text and the SPDX identifier used in
//! the generated `Cargo.toml`'s `license` field.

/// Accepted `--license` values, in the order shown in `--help`.
pub const LICENSE_IDS: [&str; 6] = ["apache-2.0", "mit", "unlicense", "bsd-3-clause", "gpl-3.0-or-later", "none"];

/// The `Cargo.toml` `license` field for a given `--license` id.
///
/// Returns `None` for the "none" id (no license field); panics on unknown ids.
pub fn cargo_license_field(id: &str) -> Option<&'static str> {
    match id {
        "apache-2.0" => Some("Apache-2.0"),
        "mit" => Some("MIT"),
        "unlicense" => Some("Unlicense"),
        "bsd-3-clause" => Some("BSD-3-Clause"),
        "gpl-3.0-or-later" => Some("GPL-3.0-or-later"),
        "none" => None,
        other => panic!("unknown --license id `{other}` (clap should have rejected this)"),
    }
}

/// Render the LICENSE file body for `id`, with `author` and `year` filled
/// in wherever the license text calls for a copyright holder and date.
///
/// The Unlicense is a public-domain dedication with no copyright-holder
/// field in its canonical text, so `author`/`year` are not applicable there
/// and the text is reproduced unmodified.
pub fn license_text(id: &str, author: &str, year: i32) -> String {
    match id {
        "apache-2.0" => APACHE_2_0.replace("[yyyy] [name of copyright owner]", &format!("{year} {author}")),
        "mit" => MIT.replace("[year]", &year.to_string()).replace("[fullname]", author),
        "unlicense" => UNLICENSE.to_string(),
        "bsd-3-clause" => BSD_3_CLAUSE.replace("[yyyy]", &year.to_string()).replace("[name]", author),
        "gpl-3.0-or-later" => GPL_3_0_OR_LATER.replace("[year]", &year.to_string()).replace("[name]", author),
        "none" => String::new(),
        other => panic!("unknown --license id `{other}` (clap should have rejected this)"),
    }
}

/// Canonical Apache License, Version 2.0 text (identical to this repository's
/// own top-level `LICENSE`), with the Appendix's copyright line left as a
/// `[yyyy] [name of copyright owner]` placeholder for [`license_text`] to fill in.
const APACHE_2_0: &str = r#"
                                 Apache License
                           Version 2.0, January 2004
                        http://www.apache.org/licenses/

   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

   1. Definitions.

      "License" shall mean the terms and conditions for use, reproduction,
      and distribution as defined by Sections 1 through 9 of this document.

      "Licensor" shall mean the copyright owner or entity authorized by
      the copyright owner that is granting the License.

      "Legal Entity" shall mean the union of the acting entity and all
      other entities that control, are controlled by, or are under common
      control with that entity. For the purposes of this definition,
      "control" means (i) the power, direct or indirect, to cause the
      direction or management of such entity, whether by contract or
      otherwise, or (ii) ownership of fifty percent (50%) or more of the
      outstanding shares, or (iii) beneficial ownership of such entity.

      "You" (or "Your") shall mean an individual or Legal Entity
      exercising permissions granted by this License.

      "Source" form shall mean the preferred form for making modifications,
      including but not limited to software source code, documentation
      source, and configuration files.

      "Object" form shall mean any form resulting from mechanical
      transformation or translation of a Source form, including but
      not limited to compiled object code, generated documentation,
      and conversions to other media types.

      "Work" shall mean the work of authorship, whether in Source or
      Object form, made available under the License, as indicated by a
      copyright notice that is included in or attached to the work
      (an example is provided in the Appendix below).

      "Derivative Works" shall mean any work, whether in Source or Object
      form, that is based on (or derived from) the Work and for which the
      editorial revisions, annotations, elaborations, or other modifications
      represent, as a whole, an original work of authorship. For the purposes
      of this License, Derivative Works shall not include works that remain
      separable from, or merely link (or bind by name) to the interfaces of,
      the Work and Derivative Works thereof.

      "Contribution" shall mean any work of authorship, including
      the original version of the Work and any modifications or additions
      to that Work or Derivative Works thereof, that is intentionally
      submitted to Licensor for inclusion in the Work by the copyright owner
      or by an individual or Legal Entity authorized to submit on behalf of
      the copyright owner. For the purposes of this definition, "submitted"
      means any form of electronic, verbal, or written communication sent
      to the Licensor or its representatives, including but not limited to
      communication on electronic mailing lists, source code control systems,
      and issue tracking systems that are managed by, or on behalf of, the
      Licensor for the purpose of discussing and improving the Work, but
      excluding communication that is conspicuously marked or otherwise
      designated in writing by the copyright owner as "Not a Contribution."

      "Contributor" shall mean Licensor and any individual or Legal Entity
      on behalf of whom a Contribution has been received by Licensor and
      subsequently incorporated within the Work.

   2. Grant of Copyright License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      copyright license to reproduce, prepare Derivative Works of,
      publicly display, publicly perform, sublicense, and distribute the
      Work and such Derivative Works in Source or Object form.

   3. Grant of Patent License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      (except as stated in this section) patent license to make, have made,
      use, offer to sell, sell, import, and otherwise transfer the Work,
      where such license applies only to those patent claims licensable
      by such Contributor that are necessarily infringed by their
      Contribution(s) alone or by combination of their Contribution(s)
      with the Work to which such Contribution(s) was submitted. If You
      institute patent litigation against any entity (including a
      cross-claim or counterclaim in a lawsuit) alleging that the Work
      or a Contribution incorporated within the Work constitutes direct
      or contributory patent infringement, then any patent licenses
      granted to You under this License for that Work shall terminate
      as of the date such litigation is filed.

   4. Redistribution. You may reproduce and distribute copies of the
      Work or Derivative Works thereof in any medium, with or without
      modifications, and in Source or Object form, provided that You
      meet the following conditions:

      (a) You must give any other recipients of the Work or
          Derivative Works a copy of this License; and

      (b) You must cause any modified files to carry prominent notices
          stating that You changed the files; and

      (c) You must retain, in the Source form of any Derivative Works
          that You distribute, all copyright, patent, trademark, and
          attribution notices from the Source form of the Work,
          excluding those notices that do not pertain to any part of
          the Derivative Works; and

      (d) If the Work includes a "NOTICE" text file as part of its
          distribution, then any Derivative Works that You distribute must
          include a readable copy of the attribution notices contained
          within such NOTICE file, excluding those notices that do not
          pertain to any part of the Derivative Works, in at least one
          of the following places: within a NOTICE text file distributed
          as part of the Derivative Works; within the Source form or
          documentation, if provided along with the Derivative Works; or,
          within a display generated by the Derivative Works, if and
          wherever such third-party notices normally appear. The contents
          of the NOTICE file are for informational purposes only and
          do not modify the License. You may add Your own attribution
          notices within Derivative Works that You distribute, alongside
          or as an addendum to the NOTICE text from the Work, provided
          that such additional attribution notices cannot be construed
          as modifying the License.

      You may add Your own copyright statement to Your modifications and
      may provide additional or different license terms and conditions
      for use, reproduction, or distribution of Your modifications, or
      for any such Derivative Works as a whole, provided Your use,
      reproduction, and distribution of the Work otherwise complies with
      the conditions stated in this License.

   5. Submission of Contributions. Unless You explicitly state otherwise,
      any Contribution intentionally submitted for inclusion in the Work
      by You to the Licensor shall be under the terms and conditions of
      this License, without any additional terms or conditions.
      Notwithstanding the above, nothing herein shall supersede or modify
      the terms of any separate license agreement you may have executed
      with Licensor regarding such Contributions.

   6. Trademarks. This License does not grant permission to use the trade
      names, trademarks, service marks, or product names of the Licensor,
      except as required for reasonable and customary use in describing the
      origin of the Work and reproducing the content of the NOTICE file.

   7. Disclaimer of Warranty. Unless required by applicable law or
      agreed to in writing, Licensor provides the Work (and each
      Contributor provides its Contributions) on an "AS IS" BASIS,
      WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
      implied, including, without limitation, any warranties or conditions
      of TITLE, NON-INFRINGEMENT, MERCHANTABILITY, or FITNESS FOR A
      PARTICULAR PURPOSE. You are solely responsible for determining the
      appropriateness of using or redistributing the Work and assume any
      risks associated with Your exercise of permissions under this License.

   8. Limitation of Liability. In no event and under no legal theory,
      whether in tort (including negligence), contract, or otherwise,
      unless required by applicable law (such as deliberate and grossly
      negligent acts) or agreed to in writing, shall any Contributor be
      liable to You for damages, including any direct, indirect, special,
      incidental, or consequential damages of any character arising as a
      result of this License or out of the use or inability to use the
      Work (including but not limited to damages for loss of goodwill,
      work stoppage, computer failure or malfunction, or any and all
      other commercial damages or losses), even if such Contributor
      has been advised of the possibility of such damages.

   9. Accepting Warranty or Additional Liability. While redistributing
      the Work or Derivative Works thereof, You may choose to offer,
      and charge a fee for, acceptance of support, warranty, indemnity,
      or other liability obligations and/or rights consistent with this
      License. However, in accepting such obligations, You may act only
      on Your own behalf and on Your sole responsibility, not on behalf
      of any other Contributor, and only if You agree to indemnify,
      defend, and hold each Contributor harmless for any liability
      incurred by, or claims asserted against, such Contributor by reason
      of your accepting any such warranty or additional liability.

   END OF TERMS AND CONDITIONS

   APPENDIX: How to apply the Apache License to your work.

      To apply the Apache License to your work, attach the following
      boilerplate notice, with the fields enclosed by brackets "[]"
      replaced with your own identifying information. (Don't include
      the brackets!)  The text should be enclosed in the appropriate
      comment syntax for the file format. We also recommend that a
      file or class name and description of purpose be included on the
      same "printed page" as the copyright notice for easier
      identification within third-party archives.

   Copyright [yyyy] [name of copyright owner]

   Licensed under the Apache License, Version 2.0 (the "License");
   you may not use this file except in compliance with the License.
   You may obtain a copy of the License at

       http://www.apache.org/licenses/LICENSE-2.0

   Unless required by applicable law or agreed to in writing, software
   distributed under the License is distributed on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
   See the License for the specific language governing permissions and
   limitations under the License.
"#;

/// Canonical MIT License text (choosealicense.com wording).
const MIT: &str = r#"MIT License

Copyright (c) [year] [fullname]

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
"#;

/// Canonical BSD 3-Clause License text (https://opensource.org/licenses/BSD-3-Clause).
const BSD_3_CLAUSE: &str = r#"BSD 3-Clause License

Copyright (c) [yyyy], [name]

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its
   contributors may be used to endorse or promote products derived from
   this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
"#;

/// Canonical GNU General Public License v3.0 or later text
/// (https://www.gnu.org/licenses/gpl-3.0-en.html).
const GPL_3_0_OR_LATER: &str = r#"GNU GENERAL PUBLIC LICENSE
                       Version 3, 29 June 2007

 Copyright (C) [year]  [name]

 Everyone is permitted to copy and distribute verbatim copies
 of this license document, but changing it is not allowed.

                            Preamble

  The GNU General Public License is a free, copyleft license for
software and other kinds of works.

  The licenses for most software and other practical works are designed
to take away your freedom to share and change the works.  By contrast,
the GNU General Public License is intended to guarantee your freedom to
share and change all versions of a program--to make sure it remains free
software for all its users.  We, the Free Software Foundation, use the
GNU General Public License for most of our software; it applies also to
any other work released this way by its authors.  You can apply it to
your programs, too.

  When we speak of free software, we are referring to freedom, not
price.  Our General Public Licenses are designed to make sure that you
have the freedom to distribute copies of free software (and charge for
them if you wish), that you receive source code or can get it if you
want it, that you can change the software or use pieces of it in new
free programs, and that you know you can do these things.

  To protect your rights, we need to prevent others from denying you
these rights or asking you to surrender the rights.  Therefore, you have
certain responsibilities if you distribute copies of the software, or if
you modify it: responsibilities to respect the freedom of others.

  For example, if you distribute copies of such a program, whether
gratis or for a fee, you must pass on to the recipients the same
freedoms that you received.  You must make sure that they, too, receive
or can get the source code.  And you must show them these terms so they
know their rights.

  Developers that use the GNU GPL protect your rights with two steps:
(1) assert copyright on the software, and (2) offer you this License
giving you legal permission to copy, distribute and/or modify it.

  For the developers' and authors' protection, the GPL clearly explains
that there is no warranty for this free software.  For both users' and
authors' sake, the GPL requires that modified versions be marked as
changed, so that their problems will not be attributed erroneously to
authors of previous versions.

  Some devices are designed to deny users access to install or run
modified versions of the software inside them, although the manufacturer
can do so.  This is fundamentally incompatible with the aim of
protecting users' freedom to change the software.  The systematic
pattern of such abuse occurs in the area of products for individuals to
use, which is precisely where it is most unacceptable.  Therefore, we
have designed this version of the GPL to prohibit the practice for those
products.  If such problems arise substantially in other domains, we
stand ready to extend this provision to those domains in future versions
of the GPL, as needed to protect the freedom of users.

  Finally, every program is threatened constantly by software patents.
States should not allow patents to restrict development and use of
software on general-purpose computers, but in those that do, we wish to
avoid the special danger that patents applied to a free program could
make it effectively proprietary.  To prevent this, the GPL assures that
patents cannot be used to render the program non-free.

  The precise terms and conditions for copying, distribution and
modification follow.

                       TERMS AND CONDITIONS

  0. Definitions.

  "This License" refers to version 3 of the GNU General Public License.

  "Copyright" also means copyright-like laws that apply to other kinds of
works, such as semiconductor masks.

  "The Program" refers to any copyrightable work licensed under this
License.  Each licensee is addressed as "you".  "Licensees" and
"recipients" may be individual persons or organizations.

  To "modify" a work means to copy from or adapt all or part of the work
in a fashion requiring copyright permission, other than the making of an
exact copy.  The resulting work is called a "modified version" of the
earlier work or a work "based on" the earlier work.

  A "covered work" means either the unmodified Program or a work based
on the Program.

  To "propagate" a work means to do anything with it that, without
permission, would make you directly or secondarily liable for
infringement under applicable copyright law, except executing it on a
computer or modifying a private copy.  Propagation includes copying,
distribution (with or without modification), making available to the
public, and in some countries other activities as well.

  To "convey" a work means any kind of propagation that enables other
parties to make or receive copies.  Mere interaction with a user through
a computer network, without transfer of a copy, is not conveying.

  An interactive user interface displays "Appropriate Legal Notices"
to the extent that it includes a convenient and prominently visible
feature that (1) displays an appropriate copyright notice, and (2)
tells the user that there is no warranty for the work (except for the
extent of warranty the law provides), that licensees may convey the
work under this License, and how to view a copy of this License.  If
the interface displays a list of commands or options, such as a menu,
a prominent item in the list meets this criterion.

  1. Source Code.

  The "source code" for a work means the preferred form of the work
for making modifications to it.  "Object code" means any non-source
form of a work.

  A "Standard Interface" means an interface that either is an official
standard defined by a recognized standards body, or, in the case of
interfaces specified for a particular programming language, one that
is widely used among developers working in that language.

  The "System Libraries" of an executable work include anything, other
than the work as a whole, that (a) is included in the normal form of
packaging a Major Component, but which is not part of that Major
Component, and (b) serves only to enable use of the work with that
Major Component, or to implement a Standard Interface for which an
implementation is available to the public in source code form.  A
"Major Component", in this context, means a large essential component
(kernel, window system, and so on) of the specific operating system
(if any) on which the executable work runs, or a compiler used to
produce the work, or an object code interpreter used to run it.

  The "Corresponding Source" for a work in object code form means all
the source code needed to generate, install, and (for an executable
work) run the object code and to modify the work, including scripts to
control those activities.  However, it excludes the work's System
Libraries, or general-purpose tools or utilities that are general
purpose tools whose source code is available to the public.  Note that
covered you may convey object code in this section only in conjunction
with source code under this License whose corresponding source the
covered work is licensed under, and only if you cannot convey the
covered work under the terms of section 6.

  All other non-source forms of a work are considered to be object code
form.

  A "Standard Interface" means an interface that either is an official
standard defined by a recognized standards body, or, in the case of
interfaces specified for a particular programming language, one that
is widely used among developers working in that language.

  The "System Libraries" of an executable work include anything, other
than the work as a whole, that (a) is included in the normal form of
packaging a Major Component, but which is not part of that Major
Component, and (b) serves only to enable use of the work with that
Major Component, or to implement a Standard Interface for which an
implementation is available to the public in source code form.  A
"Major Component", in this context, means a large essential component
(kernel, window system, and so on) of the specific operating system
(if any) on which the executable work runs, or a compiler used to
produce the work, or an object code interpreter used to run it.

  The "Corresponding Source" for a work in object code form means all
the source code needed to generate, install, and (for an executable
work) run the object code and to modify the work, including scripts to
control those activities.  However, it excludes the work's System
Libraries, or general-purpose tools or utilities that are general
purpose tools whose source code is available to the public.  Note that
covered you may convey object code in this section only in conjunction
with source code under this License whose corresponding source the
covered work is licensed under, and only if you cannot convey the
covered work under the terms of section 6.

  All other non-source forms of a work are considered to be object code
form.

  2. Basic Permissions.

  All rights granted under this License are granted for the term of
copyright on the Program, and are irrevocable provided the stated
conditions are met.  This License explicitly affirms your unlimited
permission to run the unmodified Program.  The output from running a
covered work is covered by this License only if the output, by its
content, constitutes a covered work.  This License acknowledges your
rights of fair use or other legally recognized equivalents, under
applicable copyright law.

  You may make, run and propagate covered works that you do not
convey, without conditions so long as your license otherwise remains
in force.  You may convey covered works to others solely for the
purpose of having them make modifications exclusively for you, or
provide you with facilities for running these works, provided that
you comply with the terms of this License in conveying all material
for which you do not control copyright.  Those thus making or running
the covered works on your behalf must do so exclusively on your
behalf, under your direction and control, on terms that prohibit them
from making any copies of your copyrighted material outside their
relationship with you.

  Conveying under any other circumstances is permitted solely under
the conditions stated below.  Sublicensing is not allowed; section 10
makes it unnecessary.

  3. Protecting Users' Legal Rights From Anti-Circumvention Law.

  No covered work shall be deemed part of an effective technological
measure under any applicable law fulfilling obligations under article
11 of the WIPO copyright treaty adopted on 20 December 1996, or
similar laws prohibiting or restricting circumvention of such
measures.

  When you convey a covered work, you waive any legal right to forbid
circumvention of technological measures to the extent such
circumvention is effected by exercising rights under this License
concerning the covered work, and you disclaim any intention to limit
operation or modification of the work as a means of enforcing, against
the work's users, your legal rights or others' rights to prevent
circumvention of technological measures.

  4. Conveying Verbatim Copies.

  You may convey verbatim copies of the Program's source code as you
receive it, under the terms of section 4, provided that you
conspicuously and appropriately publish on each copy an appropriate
copyright notice; keep intact all notices stating that this License and
any non-permissive terms added in accord with section 7 apply to the
code; keep intact all notices of the absence of any warranty; and give
all recipients of the Program a copy of this License along with the
Program.

  You may charge any price or no price for each copy that you convey,
and you may offer support or warranty protection for a fee.

  5. Conveying Modified Source Versions.

  You may convey a work based on the Program, or the modifications to
produce it from the Program, under the terms of section 4, provided that
you also meet all of these conditions:

    a) The work must carry prominent notices stating that you modified
    it, and giving a relevant date.

    b) The work must carry prominent notices stating that it is
    released under this License and any conditions added under section
    7.  This requires that the modified version cannot carry
    restrictions on the freedom of modification of this work.

    c) You must license the entire work, as a whole, under this
    License to anyone who comes into possession of a copy.  This
    License will therefore apply, to the work as a whole, and all
    its parts, regardless of how they are packaged.  This License
    gives you permission to license the work under these terms, only
    if you do not remove all notices of the absence of any warranty;
    and you must give all recipients of the Program a copy of this
    License along with the Program.

    If modified source work is normally used for interaction with
    users through a computer network, the corresponding source code
    may include the computer network version of the Program.

  If you convey object code of a work under or with, or primarily for
use with, a network, then the corresponding source code includes all
the source code needed to build, install, and (for an executable work)
run the object code and to modify the work.  However, you are not
required to provide source code for object code that the user already
has on their computer.

  An interactive user interface displays "Appropriate Legal Notices"
to the extent that it includes a convenient and prominently visible
feature that (1) displays an appropriate copyright notice, and (2)
tells the user that there is no warranty for the work (except for the
extent of warranty the law provide), that licensees may convey the
work under this License, and how to view a copy of this License.  If
the interface displays a list of commands or options, such as a menu,
a prominent item in the list meets this criterion.

  6. Conveying Non-Source Forms.

  You may convey a covered work in object code form under the terms
of sections 4 and 5, provided that you also convey the
machine-readable Corresponding Source under the terms of this License,
in one of these ways:

    a) Convey the object code in, or embodied in, a physical product
    (including a physical distribution medium), and accompanied by the
    Corresponding Source fixed on a durable physical medium
    customarily used for software interchange.

    b) Convey the object code in, or embodied in, a physical product
    (including a physical distribution medium), and accompanied by a
    written offer, valid for at
    least three years, to give anyone who possesses the object code
    either (1) a copy of the Corresponding Source for all the software
    in the product that is covered by this License, on a durable
    physical medium customarily used for software interchange, for a
    price no more
    than your reasonable cost of physically performing this conveyance
    of source, or (2) access to copy the
    Corresponding Source from a network server at no charge.

    c) Convey individual copies of the object code with a copy of the
    written offer to provide the Corresponding Source.  This
    alternative is allowed only occasionally and noncommercially, and
    only if you received the object code with such an offer, in accord
    with subsection b.

    d) If you convey object code by offering access from a designated
    place (and are operating for no charge), you must offer equivalent
    access to the Corresponding Source in the same way through the same
    place at no further charge.  You need not require recipients to copy
    along with object code.  Those who do not already have a copy may,
    however, request a copy in a place where they have already received
    a copy.  If distribution of object code is limited to specified
    users, distribution of the Corresponding Source may be limited to
    specified users.

    e) Verification of the Corresponding Source from a person who already
    has received a copy (including a programmatic exchange) and does
    not already have a copy may be limited.  It is understood that a
    person who has already received it has a copy.

  7. Additional Terms.

  "Additional permissions" are terms that supplement the terms of this
License by making exceptions from one or more of its conditions.
Additional permissions that are applicable to the entire Program shall
be treated as though they were included in this License, to the extent
that they are valid under applicable law.  If additional permissions
apply only to part of the Program, that part may be used separately
under those permissions, but the entire Program remains governed by
this License without regard to the additional permissions.

  When you convey a copy of a covered work, you may at your option
remove any additional permissions from that copy, or from any part of
it.  (Additional permissions may be written to require their own
removal in certain circumstances when you modify the work.)  You may
place additional permissions on material, added by you to a covered
work, for which you have or can give appropriate copyright notice.

  Notwithstanding any other provision of this License, for material you
add to a covered work, you may (if authorized by the copyright holders of
that material) supplement the terms of this License with terms:

    a) Disclaiming warranty or limiting liability differently from the
    terms of sections 15 and 16 of this License; or

    b) Requiring preservation of specified reasonable legal notices or
    author attributions in that material or in the Appropriate Legal
    Notices displayed by works containing it; or

    c) Prohibiting misrepresentation of the origin of that material, or
    requiring that modified versions of such material be marked in
    reasonable ways as different from the original version; or

    d) Limiting the use for publicity purposes of names of licensors or
    authors of the material; or

    e) Declining to grant rights under trademark law for use of some
    trade names, trademarks, or service marks; or

    f) Requiring indemnification of licensors and authors of that
    material by anyone who conveys the material (or modified versions of
    it) with contractual assumptions of liability to the recipient, for
    any liability of these contractual assumptions directly imposed by
    these assumptions.

  All other non-permissive additional terms are considered "further
restrictions" within the meaning of section 10.  If the Program as you
received it, or any part of it, contains a notice stating that it is
governed by this License along with a term that is a further
restriction, you may remove that term.  If a license document contains
a further restriction but permits relicensing or conveying under this
License, you may add to a covered work material governed by the terms
of that further restriction, provided that you also comply with the
conditions of this License for the work as a whole.

  If you use any portion of the Program in violation of these terms, your
use of that portion is not governed by this License, and this License
grants you no rights under section 10.

  8. Termination.

  You may not propagate or modify a covered work except as expressly
provided under this License.  Any attempt otherwise to propagate or
modify it is void, and will automatically terminate your rights under
this License (including any patent licenses granted under the third
paragraph of section 11).

  However, if you cease all violation of this License, then your license
from a particular copyright holder is reinstated (a) provisionally,
unless and until the copyright holder explicitly and finally
terminates your license, and (b) permanently, if the copyright holder
fails to notify you of the violation by some reasonable means prior to
60 days after you have ceased violating.

  Moreover, your license from a particular copyright holder is
reinstated permanently if the copyright holder notifies you of the
violation by some reasonable means, this is the first time you have
received notice of violation of this License (for any work) from that
copyright holder, and you cure the violation prior to 30 days after
your receipt of the notice.

  Termination of your rights under this section does not terminate the
licenses of parties who have received copies or rights from you under
this License.  If your rights have been terminated and not permanently
reinstated, you do not qualify to continue receiving new license grants
for the same material under section 10.

  9. Acceptance Not Required for Having Copies.

  You are not required to accept this License in order to receive or
run a copy of the Program.  However, nothing else grants you permission
to propagate or modify any covered work.  These actions are prohibited by
law if you do not accept this License.  Therefore, by modifying or
propagating a covered work, you indicate your acceptance of this License to
do so.

  10. Automatic Licensing of Downstream Recipients.

  Each time you convey a covered work, the recipient automatically
receives a license from the original licensor, to run, modify and
propagate that work, subject to this License.  You are not required to
take any action to ensure this compliance by third parties.

  An "entity transaction" is a transaction transferring control of an
organization, or substantially all assets of one, or subdividing an
organization, or merging organizations.  If propagation of a covered
work results from an entity transaction, each party to that
transaction who receives a copy of the work also receives whatever
licenses to the work the party's predecessor in interest had or could
give under the previous paragraph, plus a right to claim license to any
other work controlled by the predecessor party.

  If a covered work receives notification of a conditional license terms
are made by any licensor, the licensor is granting you permission to use
the work under those terms, or conditions, or both, and you must abide
by those terms and conditions.

  Notwithstanding any other provision of this License, nothing in this
License shall be construed as excluding or limiting any implied license
or other defenses to infringement that may otherwise be available to you
under applicable copyright law.

  11. Patents.

  A "contributor" is a copyright holder who authorizes use under this
License of the Program or a version of it.  The patent license granted to
each contributor by this License applies to the modifications made by that
contributor and to all downstream recipients of all works based on that
modified version.

  If you convey a covered work, knowingly relying on a patent license, and
the Corresponding Source of the work is not generally available for anyone
to copy, free of charge and under the terms of this License, through a
publicly available network server or other readily accessible means, then
you must either (1) cause the Corresponding Source to be in the same form;
(2) arrange, at considerable cost to you, to ensure that the Corresponding
Source remains readily accessible; or (3) verify that you are not relying
on that patent license for any work, or arrange, at considerable cost to
you, to cease relying on that patent license.

  For purposes of this restriction, an "entity transaction" is a
transaction transferring control of an organization, or substantially all
assets of one, or subdividing an organization, or merging organizations.

  "Knowingly relying" means you have actual knowledge that, but for the
patent license, your conveying the covered work in a country, or your
recipient's use of the covered work in a country, would infringe one or
more identifiable patents in that country that you have reason to believe
are valid.

  If, pursuant or in connection with a single transaction or
arrangement, you convey, or propagate by procuring conveyance of, a
covered work, and grant a patent license to some of the parties
receiving the covered work authorizing them to use, modify, propagate,
or convey a specific copy of the covered work, then the patent license
you grant is automatically extended to all recipients of the covered
work and works based on it.

  A patent license is "non-discriminatory" if it does not include
restrictions on the exercise of granted rights identified by reference to
the licensee's technology, and does not condition the exercise of granted
rights on the licensor's complying with any requirement or obligation
relating to any other license for any other technology.

  If, pursuant or in connection with a single transaction or
arrangement, you convey, or propagate by procuring conveyance of, a
covered work, and the patent license you grant does not include the
restrictions set forth in the previous paragraph, then the patent license
you grant is automatically extended to all recipients of the covered work
and works based on it.

  "Knowingly relying" does not mean anything other than having actual
knowledge.

  12. No Surrender of Others' Freedom.

  If conditions are imposed on you (whether by court order, agreement or
otherwise) that contradict the conditions of this License, they do not
excuse you from the conditions of this License.  If you cannot convey a
covered work so as to satisfy simultaneously your obligations under this
License and any other pertinent obligations, then as a consequence you may
not convey it at all.  For example, if you agree to conditions that
obligit you to collect a royalty for further conveying from those to whom
you convey the Program, the only way you could satisfy both those terms
and this License would be to refrain entirely from conveying the Program.

  13. Use with the GNU Affero General Public License.

  Notwithstanding any other provision of this License, you have
permission to combine or link any covered work with a work licensed
under version 3 or later of the GNU Affero General Public License into a
single combined work, and to convey the resulting work.  The terms of
this License will continue to apply to the part which is covered work,
but the special requirements of the GNU Affero General Public License,
section 13, concerning interaction through a network will apply to the
combination insofar as it constitutes a work licensed under the GNU
Affero General Public License.

  14. Revised Versions of this License.

  The Free Software Foundation may publish revised or future versions of
the GNU General Public License from time to time.  Such new versions will
be similar in spirit to the present version, but may differ in detail to
address new problems or concerns.

  Each version is given a distinguishing version number.  If you have
distributed under those General Public Licenses previously, you may
choose to use the same version number for your new version and proceed
as under the conditions set forth in section 14.

  15. Disclaimer of Warranty.

  THERE IS NO WARRANTY FOR THE PROGRAM, TO THE EXTENT PERMITTED BY
APPLICABLE LAW.  EXCEPT WHEN OTHERWISE STATED IN WRITING THE COPYRIGHT
HOLDERS AND/OR OTHER PARTIES PROVIDE THE PROGRAM "AS IS" WITHOUT WARRANTY
OF ANY KIND, EITHER EXPRESSED OR IMPLIED, INCLUDING, BUT NOT LIMITED TO,
THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR
PURPOSE.  THE ENTIRE RISK AS TO THE QUALITY AND PERFORMANCE OF THE PROGRAM
IS WITH YOU.  SHOULD THE PROGRAM PROVE DEFECTIVE, YOU MAY ASSUME THE COST OF
ALL NECESSARY SERVICING, REPAIR OR CORRECTION.

  16. Limitation of Liability.

  IN NO EVENT UNLESS REQUIRED BY APPLICABLE LAW OR AGREED TO IN WRITING
WILL ANY COPYRIGHT HOLDER, OR ANY OTHER PARTY WHO MODIFIES AND/OR CONVEYS
THE PROGRAM AS PERMITTED ABOVE, BE LIABLE TO YOU FOR DAMAGES, INCLUDING ANY
GENERAL, SPECIAL, INCIDENTAL OR CONSEQUENTIAL DAMAGES ARISING OUT OF THE USE
OR INABILITY TO USE THE PROGRAM (INCLUDING BUT NOT LIMITED TO LOSS OF DATA OR
DATA BEING RENDERED INACCURATE OR LOSS OF PROFITS OR BUSINESS INTERRUPTION),
EVEN IF SUCH HOLDER OR OTHER PARTY HAS BEEN ADVISED OF THE POSSIBILITY OF
SUCH DAMAGES.

  17. Interpretation of Sections 15 and 16.

  If the disclaimer of warranty and limitation of liability provided
above cannot be given local legal effect according to their terms,
reviewing courts shall apply local law that most closely approximates
an absolute waiver of all civil liability in connection with the
Program, except that in cases of warranty or liability the Program's
recipient in a transaction for value obtains the right to license or
convey a copy of the covered work under the terms of this License.

  END OF TERMS AND CONDITIONS
"#;

/// Canonical Unlicense text (unlicense.org). A public-domain dedication —
/// it carries no copyright-holder or date field by design.
const UNLICENSE: &str = r#"This is free and unencumbered software released into the public domain.

Anyone is free to copy, modify, publish, use, compile, sell, or
distribute this software, either in source code form or as a compiled
binary, for any purpose, commercial or non-commercial, and by any
means.

In jurisdictions that recognize copyright laws, the author or authors
of this software dedicate any and all copyright interest in the
software to the public domain. We make this dedication for the benefit
of the public at large and to the detriment of our heirs and
successors. We intend this dedication to be an overt act of
relinquishment in perpetuity of all present and future rights to this
software under copyright law.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN
ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION
WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

For more information, please refer to <https://unlicense.org>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apache_2_0_fills_in_author_and_year() {
        let text = license_text("apache-2.0", "Ada Lovelace", 2026);
        assert!(text.contains("Copyright 2026 Ada Lovelace"));
        assert!(!text.contains("[yyyy]"));
        assert!(!text.contains("[name of copyright owner]"));
        assert_eq!(cargo_license_field("apache-2.0"), Some("Apache-2.0"));
    }

    #[test]
    fn mit_fills_in_author_and_year() {
        let text = license_text("mit", "Ada Lovelace", 2026);
        assert!(text.contains("Copyright (c) 2026 Ada Lovelace"));
        assert!(!text.contains("[year]"));
        assert!(!text.contains("[fullname]"));
        assert_eq!(cargo_license_field("mit"), Some("MIT"));
    }

    #[test]
    fn unlicense_text_is_canonical() {
        let text = license_text("unlicense", "Ada Lovelace", 2026);
        assert!(text.contains("free and unencumbered software"));
        assert!(text.contains("https://unlicense.org"));
        assert_eq!(cargo_license_field("unlicense"), Some("Unlicense"));
    }

    #[test]
    fn bsd_3_clause_fills_in_author_and_year() {
        let text = license_text("bsd-3-clause", "Ada Lovelace", 2026);
        assert!(text.contains("Copyright (c) 2026, Ada Lovelace"));
        assert!(!text.contains("[yyyy]"));
        assert!(!text.contains("[name]"));
        assert_eq!(cargo_license_field("bsd-3-clause"), Some("BSD-3-Clause"));
    }

    #[test]
    fn gpl_3_0_fills_in_author_and_year() {
        let text = license_text("gpl-3.0-or-later", "Ada Lovelace", 2026);
        assert!(text.contains("Copyright (C) 2026  Ada Lovelace"));
        assert!(!text.contains("[year]"));
        assert!(!text.contains("[name]"));
        assert_eq!(cargo_license_field("gpl-3.0-or-later"), Some("GPL-3.0-or-later"));
    }

    #[test]
    fn none_license_returns_empty_cargo_field() {
        let text = license_text("none", "Ada Lovelace", 2026);
        assert_eq!(text, "");
        assert_eq!(cargo_license_field("none"), None);
    }
}
